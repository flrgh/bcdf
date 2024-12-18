use std::marker::PhantomData;

use fuzzt::algorithms::jaro;
use unicode_normalization::UnicodeNormalization;

use crate::types::{self, SpotifyTrack};

const TITLE_WEIGHT: f64 = 100.0;
const ARTIST_WEIGHT: f64 = 50.0;
const ALBUM_WEIGHT: f64 = 10.0;
const DURATION_WEIGHT: f64 = 5.0;
const TRACKNUM_WEIGHT: f64 = 5.0;

const MIN_TITLE_SCORE: f64 = 95.0;
const MIN_ARTIST_SCORE: f64 = 90.0;

fn normalize(s: &str) -> String {
    fn replace_equivalent_char(c: char) -> char {
        const REPLACE: &[(&[char], char)] = &[
            (&['—'], '-'),
            (&['“', '”'], '"'),
            (&['‘', '’'], '\''),
            (&['`', '´'], '\''),
            (&['í'], 'i'),
        ];

        for (find, repl) in REPLACE {
            if find.contains(&c) {
                return *repl;
            }
        }

        c
    }

    fn keep_char(c: &char) -> bool {
        const REMOVE: &[char] = &['(', ')', '[', ']'];
        !REMOVE.contains(c)
    }

    fn strip_suffix(s: &str) -> String {
        const STRIP_SUFFIXES: &[char] = &['!', '.', '?'];
        s.strip_suffix(STRIP_SUFFIXES).unwrap_or(s).to_string()
    }

    fn keep_segment(s: &&str) -> bool {
        const DROP_SEGMENTS: &[&str] = &["-", "/", ":"];
        !DROP_SEGMENTS.contains(s)
    }

    fn replace_segment(s: &str) -> &str {
        const REPLACE_SEGMENTS: &[(&str, &str)] = &[("&", "and"), ("feat.", "feat")];

        for (find, repl) in REPLACE_SEGMENTS {
            if *find == s {
                return repl;
            }
        }

        s
    }

    fn is_segment_separator(c: char) -> bool {
        match c {
            '/' => true, // split "a/b" => "a b"
            c => c.is_whitespace(),
        }
    }

    fn trim_empty_segments(s: &str) -> Option<&str> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        Some(s)
    }

    let s = s.nfc().collect::<String>().to_lowercase();

    if s.len() <= 8 {
        return s;
    }

    let input = &s;

    let s = strip_suffix(&s);

    let normalized = s
        .chars()
        .map(replace_equivalent_char)
        .filter(keep_char)
        .collect::<String>()
        .split(is_segment_separator)
        .filter_map(trim_empty_segments)
        .filter(keep_segment)
        .map(replace_segment)
        .collect::<Vec<&str>>()
        .join(" ");

    // did normalization produce a radically different value from the input?
    let diff = (1.0 - jaro(input, &normalized)) * 100.0;
    if diff > 50.0 {
        tracing::warn!("normalize('{input}') => '{normalized}' with a difference of {diff}");
    }

    normalized
}

#[derive(Debug, Clone)]
struct TrackTitle;
#[derive(Debug, Clone)]
struct Artist;
#[derive(Debug, Clone)]
struct Album;

trait MatchType {
    fn label() -> &'static str {
        let ty = std::any::type_name::<Self>();

        // extract the last bit of the type name
        //
        // bcdf::search::TrackTitle => TrackTitle
        if let Some(pos) = ty.rfind("::") {
            return &ty[pos + 2..];
        }
        ty
    }
}

impl MatchType for TrackTitle {}
impl MatchType for Artist {}
impl MatchType for Album {}

#[derive(Debug)]
struct StringMatcher<MT: MatchType> {
    original: String,
    normalized: String,
    _mt: PhantomData<MT>,
}

impl<MT: MatchType> StringMatcher<MT> {
    fn new(s: &str) -> Self {
        let original = s.to_string();
        let normalized = normalize(s);

        Self {
            original,
            normalized,
            _mt: Default::default(),
        }
    }

    fn score(&mut self, s: &str) -> f64 {
        let norm = normalize(s);

        let score = if self.original.eq_ignore_ascii_case(s) {
            tracing::debug!("exact match for {} '{}'", MT::label(), s);
            100.0f64
        } else {
            jaro(&self.normalized, &norm) * 100f64
        };

        tracing::debug!(
            "search({}) subject('{}'), candidate('{}') => {}%",
            MT::label(),
            self.normalized,
            norm,
            score,
        );

        score
    }
}

#[derive(Debug, Clone)]
struct TrackNumMatcher {
    num: usize,
}

impl TrackNumMatcher {
    fn new(num: usize) -> Self {
        Self { num }
    }

    fn score(&self, other: usize) -> f64 {
        if self.num == other {
            100.0
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone)]
struct TrackDurationMatcher {
    duration: u64,
}

impl TrackDurationMatcher {
    fn new(duration: u64) -> Self {
        Self { duration }
    }

    fn score(&self, other: u64) -> f64 {
        let diff = self.duration.abs_diff(other);
        let percent = (1 - (diff / self.duration)) * 100;
        assert!((0..=100).contains(&percent));
        percent as f64
    }
}

struct MatchParams<'a> {
    title: &'a str,
    artist: Vec<&'a str>,
    album: &'a str,
    number: usize,
    duration: u64,
}

impl<'a> From<&'a types::SpotifyTrack> for MatchParams<'a> {
    fn from(value: &'a types::SpotifyTrack) -> MatchParams<'a> {
        Self {
            title: &value.name,
            artist: value.artists.iter().map(|a| a.name.as_str()).collect(),
            album: &value.album.name,
            number: value.track_number as usize,
            duration: value.duration.num_seconds() as u64,
        }
    }
}

impl<'a> From<&'a (types::Track, types::Track)> for MatchParams<'a> {
    fn from(value: &'a (types::Track, types::Track)) -> MatchParams<'a> {
        let (search_result, _) = value;
        search_result.into()
    }
}

impl<'a> From<&'a types::Track> for MatchParams<'a> {
    fn from(value: &'a types::Track) -> MatchParams<'a> {
        Self {
            title: &value.title,
            artist: vec![&value.artist.name],
            album: &value.album.title,
            number: value.number,
            duration: value.duration.as_secs(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct TrackMatcher<'a> {
    track: &'a crate::types::Track,
    title: StringMatcher<TrackTitle>,
    artist: StringMatcher<Artist>,
    album: StringMatcher<Album>,
    number: TrackNumMatcher,
    duration: TrackDurationMatcher,
}

impl<T, MT> From<T> for StringMatcher<MT>
where
    T: AsRef<str>,
    MT: MatchType,
{
    fn from(value: T) -> Self {
        Self::new(value.as_ref())
    }
}

impl<'a> TrackMatcher<'a> {
    pub(crate) fn new(track: &'a types::Track) -> anyhow::Result<TrackMatcher<'a>> {
        Ok(Self {
            track,
            title: StringMatcher::new(&track.title),
            artist: StringMatcher::new(&track.artist.name),
            album: StringMatcher::new(&track.album.title),
            number: TrackNumMatcher::new(track.number),
            duration: TrackDurationMatcher::new(track.duration.as_secs()),
        })
    }

    fn title_score(&mut self, result: &MatchParams) -> f64 {
        self.title.score(result.title)
    }

    fn album_score(&mut self, result: &MatchParams) -> f64 {
        self.album.score(result.album)
    }

    fn artist_score(&mut self, result: &MatchParams) -> f64 {
        let matched = if result.artist.len() > 1 {
            let candidates: Vec<String> = {
                let joined = {
                    let mut list = result.artist.clone();
                    list.sort();
                    list.join(" & ")
                };

                result
                    .artist
                    .iter()
                    .map(|s| s.to_string())
                    .chain(std::iter::once(joined))
                    .collect()
            };

            let subject = {
                let mut artist: Vec<_> = self
                    .artist
                    .original
                    .split_whitespace()
                    .filter_map(|word| {
                        let word = word.to_lowercase();
                        match word.as_str() {
                            "feat" | "featuring" | "feat." | "and" | "&" | "w/" => None,
                            _ => Some(word),
                        }
                    })
                    .collect();

                artist.sort();
                artist.join(" & ")
            };

            let mut matcher = StringMatcher::<Artist>::new(&subject);

            candidates
                .iter()
                .map(|artist| {
                    let score = matcher.score(artist);
                    (score, artist.clone())
                })
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
        } else {
            result
                .artist
                .iter()
                .map(|artist| {
                    let score = self.artist.score(artist);
                    (score, artist.to_string())
                })
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
        };

        match matched {
            Some((score, artist)) => {
                tracing::debug!("artist: {}, score: {}%", artist, score);
                score
            }
            None => 0f64,
        }
    }

    fn track_number_score(&self, result: &MatchParams) -> f64 {
        self.number.score(result.number)
    }

    fn duration_score(&self, result: &MatchParams) -> f64 {
        self.duration.score(result.duration)
    }

    pub(crate) fn score(&mut self, result: &SpotifyTrack) -> Option<u64> {
        let params = MatchParams::from(result);
        self.score_params(params)
    }

    fn score_params(&mut self, result: MatchParams) -> Option<u64> {
        let title = self.title_score(&result);
        let artist = self.artist_score(&result);
        let album = self.album_score(&result);
        let tracknum = self.track_number_score(&result);
        let duration = self.duration_score(&result);

        let score = (title * TITLE_WEIGHT)
            + (artist * ARTIST_WEIGHT)
            + (album * ALBUM_WEIGHT)
            + (tracknum * TRACKNUM_WEIGHT)
            + (duration * DURATION_WEIGHT);

        let comp = (score / self.max_possible() as f64) * 100.0;
        tracing::debug!("composite score: {comp}");

        if title < MIN_TITLE_SCORE || artist < MIN_ARTIST_SCORE {
            return None;
        }

        Some(comp.floor() as u64)
    }

    fn max_possible(&self) -> u64 {
        (TITLE_WEIGHT as u64
            + ARTIST_WEIGHT as u64
            + ALBUM_WEIGHT as u64
            + TRACKNUM_WEIGHT as u64
            + DURATION_WEIGHT as u64)
            * 100
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_search() {
        let cases = vec![
            // (spotify result string, bandcamp title)
            ("hey, ily", "Hey, ily!"),
            ("for toshiko: ii. to touch —", "for Toshiko, ii. to touch—"),
            ("onward! to nowhere", "DISKORD - Onward! To Nowhere"),
            ("desiree", "desirée"),
            (
                "you cant negotiate with zombies",
                "You Can't Negotiate With Zombies [Debut Album]",
            ),
            (
                "are there not still fireflies",
                "Are There Not Still Fireflies?",
            ),
            ("laurie anderson", "Anne Waldman, Laurie Anderson"),
            ("nativo vol. 1", "[NTV001] NATIVO VA VOL 1."),
        ];

        for (search_result, bandcamp_title) in cases {
            // technically not all of our tests are track titles, but that
            // doesn't matter here.
            let mut matcher: StringMatcher<TrackTitle> = StringMatcher::new(bandcamp_title);
            let result = matcher.score(search_result);
            assert!(
                result > 0.0,
                "search for '{search_result}' in '{bandcamp_title}' yielded a zero score"
            );
        }
    }

    #[test]
    fn track_matcher_exact() {
        let track = {
            let mut track = types::Track::new("track", "artist", "album");
            track.duration = types::Duration::from_secs(30);
            track.number = 2;
            track
        };

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        let score = matcher.score_params((&track).into());

        assert_eq!(Some(100), score);
    }

    #[test]
    fn track_matcher_similar_title() {
        let track = {
            let mut track = types::Track::new("my track name!!", "artist", "album");
            track.duration = types::Duration::from_secs(30);
            track.number = 2;
            track
        };

        let other = {
            let mut other = track.clone();
            other.title = format!("{}!", other.title);
            other
        };

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        let score = matcher.score_params((&other).into());

        assert_eq!(Some(98), score);
    }

    #[test]
    fn track_matcher_wrong_title() {
        let track = {
            let mut track = types::Track::new("title", "artist", "album");
            track.duration = types::Duration::from_secs(30);
            track.number = 2;
            track
        };

        let other = {
            let mut other = track.clone();
            other.title = "nope nope bad title".to_string();
            other
        };

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        let score = matcher.score_params((&other).into());

        assert_eq!(None, score);
    }

    #[test]
    fn track_matcher_wrong_artist() {
        let track = {
            let mut track = types::Track::new("title", "artist", "album");
            track.duration = types::Duration::from_secs(30);
            track.number = 2;
            track
        };

        let other = {
            let mut other = track.clone();
            other.artist = types::Artist::new("nope not the right artist");
            other
        };

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        let score = matcher.score_params((&other).into());

        assert_eq!(None, score);
    }

    #[test]
    fn track_matcher_multi_artist() {
        let tests = &[
            ("a & b", vec!["a", "b"]),
            ("a and b", vec!["a", "b"]),
            ("a & b & c", vec!["a", "b", "c"]),
            ("a & b & c", vec!["b", "c", "a"]),
            ("b & c & a", vec!["a", "b", "c"]),
            ("b feat. a", vec!["b", "a"]),
            ("b feat. a", vec!["a", "b"]),
        ];

        for case in tests {
            let track = {
                let mut track = types::Track::new("title", case.0, "album");
                track.duration = types::Duration::from_secs(30);
                track.number = 2;
                track
            };

            let mut params = MatchParams::from(&track);
            params.artist = case.1.clone();

            let mut matcher = TrackMatcher::new(&track).expect("should not fail");

            let score = matcher.score_params(params);

            assert_eq!(
                Some(100),
                score,
                "track: '{}', result: '{:?}'",
                case.0,
                case.1
            );
        }
    }
}
