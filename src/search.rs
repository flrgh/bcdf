use std::marker::PhantomData;

use chrono::TimeDelta;
use fuzzt::algorithms::jaro;
use unicode_normalization::UnicodeNormalization;

use crate::types;

const TITLE_WEIGHT: f64 = 100.0;
const ARTIST_WEIGHT: f64 = 50.0;
const ALBUM_WEIGHT: f64 = 10.0;
const DURATION_WEIGHT: f64 = 5.0;
const TRACKNUM_WEIGHT: f64 = 5.0;

const MIN_TITLE_SCORE: f64 = 95.0;
const MIN_ARTIST_SCORE: f64 = 90.0;

type SpotifyTrack = rspotify::model::FullTrack;

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

#[derive(Debug)]
pub(crate) struct TrackMatcher {
    title: StringMatcher<TrackTitle>,
    artist: StringMatcher<Artist>,
    album: StringMatcher<Album>,
    number: usize,
    duration: TimeDelta,
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

impl TrackMatcher {
    pub(crate) fn new(track: &types::Track) -> anyhow::Result<Self> {
        Ok(Self {
            title: StringMatcher::new(&track.title),
            artist: StringMatcher::new(&track.artist.name),
            album: StringMatcher::new(&track.album.title),
            number: track.number,
            duration: TimeDelta::from_std(track.duration)?,
        })
    }

    fn title_score(&mut self, result: &SpotifyTrack) -> f64 {
        self.title.score(&result.name)
    }

    fn album_score(&mut self, result: &SpotifyTrack) -> f64 {
        self.album.score(&result.album.name)
    }

    fn artist_score(&mut self, result: &SpotifyTrack) -> f64 {
        let res = result
            .artists
            .iter()
            .map(|art| (art.name.to_string(), self.artist.score(&art.name)))
            .max_by(|a, b| (a.1).partial_cmp(&b.1).unwrap());

        match res {
            Some((artist, score)) => {
                tracing::debug!("artist: {}, score: {}%", artist, score);

                score
            }
            None => 0f64,
        }
    }

    pub(crate) fn score(&mut self, result: &SpotifyTrack) -> Option<u64> {
        let title = self.title_score(result);
        let artist = self.artist_score(result);
        let album = self.album_score(result);

        let mut tracknum = 0f64;
        if album > MIN_TITLE_SCORE && result.track_number == (self.number as u32) {
            tracknum = 100.0;
        }

        let duration = {
            let percent = 100
                - (self.duration - result.duration).num_seconds().abs()
                    / self.duration.num_seconds();
            assert!((0..=100).contains(&percent));
            percent as f64
        };

        let score = (title * TITLE_WEIGHT)
            + (artist * ARTIST_WEIGHT)
            + (album * ALBUM_WEIGHT)
            + (tracknum * TRACKNUM_WEIGHT)
            + (duration * DURATION_WEIGHT);

        let comp = (score as f32 / self.max_possible() as f32) * 100.0;
        tracing::debug!("composite score: {comp}");

        if title < MIN_TITLE_SCORE || artist < MIN_ARTIST_SCORE {
            return None;
        }

        Some(comp.round() as u64)
    }

    fn max_possible(&self) -> u64 {
        (TITLE_WEIGHT as u64 + ARTIST_WEIGHT as u64 + ALBUM_WEIGHT as u64 + TRACKNUM_WEIGHT as u64)
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
}
