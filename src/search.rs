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

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .replace(['“', '”'], "\"")
        .split(" ")
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
    fn new(score: u16, max: u16) -> Self {
        Self {
            score,
            max,
            weight: 1,
        }
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

    fn with_weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    fn weighted(&self) -> u32 {
        self.score as u32 * self.weight as u32
    }
}

impl Ord for MatchResult {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        assert!(self.max == other.max);
        self.score.cmp(&other.score)
    }
}

#[derive(Debug)]
struct StringMatcher {
    matcher: Matcher,
    atom: Atom,
    buf: Vec<char>,
    max: u16,
    original: String,
    normalized: String,
}

impl StringMatcher {
    fn new(s: &str) -> Self {
        let original = s.to_string();
        let normalized = normalize(s);
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
            let max = atom
                .score(haystack, &mut matcher)
                .expect("wtf this should always match");
            assert!(max > 0);

            max
        };

        Self {
            matcher,
            atom,
            buf,
            max,
            normalized,
            original,
        }
    }

    fn score(&mut self, s: &str) -> MatchResult {
        if self.original == s {
            println!("exact match for '{}'", s);
            return MatchResult::new(self.max, self.max);
        }

        let haystack = Utf32Str::new(s, &mut self.buf);
        let score = self.atom.score(haystack, &mut self.matcher).unwrap_or(0);

        let mr = MatchResult::new(score, self.max);

        println!(
            "search '{}' in '{}': {}/{} ({})",
            s,
            self.original,
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
}

impl TrackMatcher {
    pub(crate) fn new(track: &types::Track) -> Self {
        Self {
            title: StringMatcher::new(&track.title),
            artist: StringMatcher::new(&track.artist.name),
            album: StringMatcher::new(&track.album.title),
            number: track.number,
        }
    }

    fn title_score(&mut self, result: &rspotify::model::FullTrack) -> MatchResult {
        self.title.score(&result.name).with_weight(TITLE_WEIGHT)
    }

    fn album_score(&mut self, result: &rspotify::model::FullTrack) -> MatchResult {
        self.album
            .score(&result.album.name)
            .with_weight(ALBUM_WEIGHT)
    }

    fn artist_score(&mut self, result: &rspotify::model::FullTrack) -> MatchResult {
        result
            .artists
            .iter()
            .map(|art| self.artist.score(&art.name))
            .max()
            .unwrap_or_else(|| MatchResult::unmatched(self.artist.max))
            .with_weight(ARTIST_WEIGHT)
    }

    pub(crate) fn score(&mut self, result: &rspotify::model::FullTrack) -> Option<u32> {
        let title = self.title_score(result);

        if title.percent() < MATCH_SCORE {
            return None;
        }

        let album = self.album_score(result);
        let artist = self.artist_score(result);

        println!(
            "track: {}, score: {}/{} ({})",
            result.name,
            title.score,
            self.title.max,
            title.percent(),
        );
        println!(
            "album: {}, score: {}/{} ({})",
            result.album.name,
            album.score,
            self.album.max,
            album.percent(),
        );

        let mut tracknum = MatchResult::new(0, 100).with_weight(TRACKNUM_WEIGHT);
        if album.percent() > MATCH_SCORE && result.track_number == (self.number as u32) {
            tracknum.score = 100;
        }

        let score = title.weighted() + artist.weighted() + album.weighted() + tracknum.weighted();

        println!("composite score: {}/{}", score, self.max_possible());

        Some(score)
    }

    fn max_possible(&self) -> u32 {
        (self.title.max as u32 * TITLE_WEIGHT as u32)
            + (self.artist.max as u32 * ARTIST_WEIGHT as u32)
            + (self.album.max as u32 * ALBUM_WEIGHT as u32)
            + TRACKNUM_WEIGHT as u32
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResultScore {
    title_match: bool,
    artist_match: bool,
    album_match: bool,
    number_match: bool,
    duration_diff: TimeDelta,
}

impl std::fmt::Display for ResultScore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Title: {}\nArtist: {}\nAlbum: {}\nNumber: {}\nDuration: {}",
            self.title_match,
            self.artist_match,
            self.album_match,
            self.number_match,
            self.duration_diff.is_zero()
        )
    }
}

impl ResultScore {
    pub(crate) fn new(track: &types::Track, result: &rspotify::model::FullTrack) -> Self {
        let artist = result
            .artists
            .iter()
            .find(|&art| art.name == track.artist.name)
            .map(|art| art.name.clone());

        let album_match = track.album.title == result.album.name;

        let diff = (TimeDelta::from_std(track.duration).unwrap() - result.duration).abs();

        let rs = Self {
            title_match: track.title == result.name,
            artist_match: artist.is_some(),
            album_match: album_match,
            number_match: album_match && (track.number as u32 == result.track_number),
            duration_diff: (TimeDelta::from_std(track.duration).unwrap() - result.duration).abs(),
        };

        println!(
            "Result: {}, artist: {}, album: {}, # {}, score: {}",
            result.name,
            artist.unwrap_or(String::from("NOMATCH")),
            result.album.name,
            result.track_number,
            rs.score(),
        );

        println!(
            "\ttitle match: {}\n\tartist match: {}\n\talbum match: {}\n\ttrackno match: {}\n\tduration diff: {}",
            rs.title_match, rs.artist_match, rs.album_match, rs.number_match,
            rs.duration_diff.num_seconds(),
        );

        rs
    }

    pub(crate) fn score(&self) -> u32 {
        let mut score = 100;

        const TITLE_WEIGHT: u32 = 100;
        const ARTIST_WEIGHT: u32 = 50;
        const ALBUM_WEIGHT: u32 = 50;
        const NUMBER_WEIGHT: u32 = 20;
        const DURATION_WEIGHT: u32 = 50;

        if self.title_match {
            score += TITLE_WEIGHT;
        }

        if self.artist_match {
            score += ARTIST_WEIGHT;
        }

        if self.album_match {
            score += ALBUM_WEIGHT;
        }

        if self.number_match && self.album_match {
            score += NUMBER_WEIGHT;
        }

        if self.duration_diff.is_zero() {
            score += DURATION_WEIGHT;
        }

        score
    }
}
