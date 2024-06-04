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
    s.to_lowercase()
        .replace(['“', '”'], "\"")
        .replace('’', "'")
        .replace(['(', ')'], "")
        .split(|c: char| c.is_whitespace())
        .filter(|s| {
            let s = s.trim();
            !s.is_empty() && s != "-"
        })
        .collect::<Vec<&str>>()
        .join(" ")
}

#[derive(Eq, PartialEq, PartialOrd)]
struct MatchResult<const W: u16 = 1> {
    score: u16,
    max: u16,
}

impl<const W: u16> MatchResult<W> {
    const WEIGHT: u16 = W;

    fn new(score: u16, max: u16) -> Self {
        Self { score, max }
    }

    fn unmatched(max: u16) -> Self {
        Self::new(0, max)
    }

    fn percent(&self) -> f32 {
        assert!(self.score <= self.max);

        if self.score == self.max {
            return 100f32;
        }

        (self.score as f32 / self.max as f32) * 100f32
    }

    fn weighted(&self) -> u32 {
        (self.percent().round() as u32) * (Self::WEIGHT as u32)
    }
}

impl Ord for MatchResult {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        assert!(self.max == other.max);
        self.score.cmp(&other.score)
    }
}

#[derive(Debug)]
struct StringMatcher<const W: u16 = 1> {
    matcher: Matcher,
    atom: Atom,
    buf: Vec<char>,
    max: u16,
    original: String,
    normalized: String,
}

impl<const W: u16> StringMatcher<W> {
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
                tracing::warn!("wtf, search for '{}' is bugged", s);
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
        }
    }

    fn score(&mut self, s: &str) -> MatchResult<W> {
        if self.original == s {
            tracing::info!("exact match for '{}'", s);
            return MatchResult::new(self.max, self.max);
        }

        let s = normalize(s);
        let haystack = Utf32Str::new(&s, &mut self.buf);
        let score = self.atom.score(haystack, &mut self.matcher).unwrap_or(0);

        let mr = MatchResult::new(score, self.max);

        tracing::info!(
            "search '{}' in '{}' (normalized: '{}'): {}/{} ({})",
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
    title: StringMatcher,
    artist: StringMatcher,
    album: StringMatcher,
    number: usize,
    duration: TimeDelta,
}

impl TrackMatcher {
    pub(crate) fn new(track: &types::Track) -> Self {
        Self {
            title: StringMatcher::new(&track.title),
            artist: StringMatcher::new(&track.artist.name),
            album: StringMatcher::new(&track.album.title),
            number: track.number,
            duration: TimeDelta::from_std(track.duration).unwrap(),
        }
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
            .unwrap_or_else(|| MatchResult::unmatched(self.artist.max))
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

        let mut tracknum: MatchResult<TRACKNUM_WEIGHT> = MatchResult::new(0, 100);
        if album.percent() > MATCH_SCORE && result.track_number == (self.number as u32) {
            tracknum.score = 100;
        }

        let mut duration: MatchResult<DURATION_WEIGHT> = MatchResult::new(0, 100);
        {
            let percent = 100 - (self.duration - result.duration).num_seconds().abs() / self.duration.num_seconds();
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
