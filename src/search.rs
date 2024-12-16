use std::marker::PhantomData;

use chrono::TimeDelta;
use nucleo_matcher::{
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
    Config, Matcher, Utf32Str,
};

use crate::types;

const TITLE_WEIGHT: u16 = 1000;
const ARTIST_WEIGHT: u16 = 500;
const ALBUM_WEIGHT: u16 = 100;
const DURATION_WEIGHT: u16 = 50;
const TRACKNUM_WEIGHT: u16 = 50;

const MATCH_SCORE: f32 = 90.0;

type SpotifyTrack = rspotify::model::FullTrack;

fn normalize(s: &str) -> String {
    let s = if s.len() > 3 {
        s.strip_suffix('.').unwrap_or(s)
    } else {
        s
    };

    s.to_lowercase()
        .replace(['“', '”', '"', '’', '\'', '(', ')', '`', '´', '[', ']'], "")
        .split(|c: char| match c {
            '/' => true, // split "a/b" => "a b"
            c => c.is_whitespace(),
        })
        .filter_map(|s| {
            let s = s.trim();

            if s.is_empty() {
                return None;
            }

            match s {
                "-" | "/" | ":" => None,
                "&" => Some("and"),
                _ => Some(s),
            }
        })
        .collect::<Vec<&str>>()
        .join(" ")
}

#[derive(Eq, PartialEq, PartialOrd)]
struct MatchResult {
    score: u16,
    max: u16,
    weight: u16,
}

impl MatchResult {
    fn new(score: u16, max: u16, weight: u16) -> Self {
        Self { score, max, weight }
    }

    fn unmatched(max: u16, weight: u16) -> Self {
        Self::new(0, max, weight)
    }

    fn percent(&self) -> f32 {
        assert!(self.score <= self.max);

        if self.score == self.max {
            return 100f32;
        }

        (self.score as f32 / self.max as f32) * 100f32
    }

    fn weighted(&self) -> u32 {
        (self.percent().round() as u32) * (self.weight as u32)
    }
}

impl Ord for MatchResult {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        assert!(self.max == other.max);
        self.score.cmp(&other.score)
    }
}

#[derive(Debug, Clone)]
struct TrackTitle;
#[derive(Debug, Clone)]
struct Artist;
#[derive(Debug, Clone)]
struct Album;

trait MatchType {
    const WEIGHT: u16;

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

    fn weight() -> u16 {
        Self::WEIGHT
    }
}

impl MatchType for TrackTitle {
    const WEIGHT: u16 = TITLE_WEIGHT;
}

impl MatchType for Artist {
    const WEIGHT: u16 = ARTIST_WEIGHT;
}

impl MatchType for Album {
    const WEIGHT: u16 = ALBUM_WEIGHT;
}

#[derive(Debug)]
struct StringMatcher<MT: MatchType> {
    matcher: Matcher,
    atom: Atom,
    buf: Vec<char>,
    max: u16,
    original: String,
    normalized: String,
    _mt: PhantomData<MT>,
}

impl<MT: MatchType> StringMatcher<MT> {
    fn new(s: &str) -> Self {
        let original = s.to_string();
        let normalized = normalize(s);
        // FIXME: Evidently, I am not using this correctly, because it rarely
        // ever works, even for strings that are trivially different :(
        let atom = Atom::new(
            &normalized,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
            true,
        );

        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buf = Vec::new();

        let max = {
            let haystack = Utf32Str::new(&normalized, &mut buf);
            let max = atom.score(haystack, &mut matcher).unwrap_or_else(|| {
                tracing::warn!("wtf, search for {} '{}' is bugged", MT::label(), s);
                u16::MAX
            });
            assert!(max > 0);

            max
        };

        Self {
            matcher,
            atom,
            buf,
            max,
            original,
            normalized,
            _mt: Default::default(),
        }
    }

    fn score(&mut self, s: &str) -> MatchResult {
        if self.original == s {
            tracing::info!("exact match for {} '{}'", MT::label(), s);
            return MatchResult::new(self.max, self.max, MT::weight());
        }

        let s = normalize(s);
        let haystack = Utf32Str::new(&s, &mut self.buf);
        let score = self.atom.score(haystack, &mut self.matcher).unwrap_or(0);

        let mr = MatchResult::new(score, self.max, MT::weight());

        tracing::info!(
            "search {} '{}' in '{}' (normalized: '{}'): {}/{} ({})",
            MT::label(),
            s,
            self.original,
            self.normalized,
            mr.score,
            self.max,
            mr.percent(),
        );

        mr
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

    fn title_score(&mut self, result: &SpotifyTrack) -> MatchResult {
        self.title.score(&result.name)
    }

    fn album_score(&mut self, result: &SpotifyTrack) -> MatchResult {
        self.album.score(&result.album.name)
    }

    fn artist_score(&mut self, result: &SpotifyTrack) -> MatchResult {
        result
            .artists
            .iter()
            .map(|art| self.artist.score(&art.name))
            .max()
            .unwrap_or_else(|| MatchResult::unmatched(self.artist.max, Artist::weight()))
    }

    pub(crate) fn score(&mut self, result: &SpotifyTrack) -> Option<u32> {
        let title = self.title_score(result);

        if title.percent() < MATCH_SCORE {
            return None;
        }

        let album = self.album_score(result);
        let artist = self.artist_score(result);

        tracing::info!(
            "track: {}, score: {}/{} ({})",
            result.name,
            title.score,
            self.title.max,
            title.percent(),
        );
        tracing::info!(
            "album: {}, score: {}/{} ({})",
            result.album.name,
            album.score,
            self.album.max,
            album.percent(),
        );

        let mut tracknum = MatchResult::new(0, 100, TRACKNUM_WEIGHT);
        if album.percent() > MATCH_SCORE && result.track_number == (self.number as u32) {
            tracknum.score = 100;
        }

        let mut duration = MatchResult::new(0, 100, DURATION_WEIGHT);
        {
            let percent = 100
                - (self.duration - result.duration).num_seconds().abs()
                    / self.duration.num_seconds();
            assert!((0..=100).contains(&percent));
            duration.score = percent as u16;
        }

        let score = title.weighted()
            + artist.weighted()
            + album.weighted()
            + tracknum.weighted()
            + duration.weighted();

        tracing::info!("composite score: {}/{}", score, self.max_possible());

        Some(score)
    }

    fn max_possible(&self) -> u32 {
        (TITLE_WEIGHT as u32 + ARTIST_WEIGHT as u32 + ALBUM_WEIGHT as u32 + TRACKNUM_WEIGHT as u32)
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
                result.score > 0,
                "search for '{search_result}' in '{bandcamp_title}' yielded a zero score"
            );
        }
    }
}
