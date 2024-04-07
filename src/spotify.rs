use std::cmp;

use chrono::TimeDelta;
use rspotify::model::{Country, IncludeExternal, Market, SearchResult, SearchType};
use rspotify::prelude::*;
use rspotify::{ClientCredsSpotify, Credentials};

use nucleo_matcher::{
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
    Config, Matcher, Utf32Str,
};

use crate::types;

type Client = ClientCredsSpotify;

const MARKET: Market = Market::Country(Country::UnitedStates);

fn normalize(s: &str) -> String {
    s.to_lowercase().split(" ").collect::<Vec<&str>>().join(" ")
}

#[derive(Debug)]
struct StringMatcher {
    matcher: Matcher,
    atom: Atom,
    buf: Vec<char>,
    max: u16,
}

impl StringMatcher {
    fn new(s: &str) -> Self {
        let s = normalize(s);
        let atom = Atom::new(
            &s,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
            true,
        );

        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buf = Vec::new();

        let max = {
            let haystack = Utf32Str::new(&s, &mut buf);
            atom.score(haystack, &mut matcher)
                .expect("wtf this should always match")
        };

        Self {
            matcher,
            atom,
            buf,
            max,
        }
    }

    fn score(&mut self, s: &str) -> u16 {
        let haystack = Utf32Str::new(s, &mut self.buf);
        let score = self.atom.score(haystack, &mut self.matcher);

        score.unwrap_or(0)
    }
}

#[derive(Debug)]
struct TrackMatcher {
    title: StringMatcher,
    artist: StringMatcher,
    album: StringMatcher,
    number: usize,
}

impl TrackMatcher {
    fn new(track: &types::Track) -> Self {
        Self {
            title: StringMatcher::new(&track.title),
            artist: StringMatcher::new(&track.artist.name),
            album: StringMatcher::new(&track.album.title),
            number: track.number,
        }
    }

    fn score(&mut self, result: &rspotify::model::FullTrack) -> u16 {
        let artist = result
            .artists
            .iter()
            .map(|art| {
                let s = self.artist.score(&art.name);
                println!("artist: {}, score: {}/{}", art.name, s, self.artist.max);
                s
            })
            .max()
            .unwrap_or(0);

        let title = self.title.score(&result.name);
        let album = self.album.score(&result.album.name);

        println!(
            "track: {}, score: {}/{}",
            result.name, title, self.title.max
        );
        println!(
            "album: {}, score: {}/{}",
            result.album.name, album, self.album.max
        );

        let mut tracknum = 0;
        if (self.album.max - album) < 50 && result.track_number == (self.number as u32) {
            tracknum = 255;
        }

        let score = (title * 10) + (artist * 8) + (album * 5) + tracknum;

        println!("composite score: {}/{}", score, self.max_possible());

        score
    }

    fn max_possible(&self) -> u16 {
        (self.title.max * 10) + (self.artist.max * 8) + (self.album.max * 5) + (255 * 1)
    }
}

pub(crate) async fn connect() -> Client {
    let creds = Credentials::from_env().unwrap();
    let spotify = ClientCredsSpotify::new(creds);
    spotify.request_token().await.unwrap();
    spotify
}

#[derive(Debug, PartialEq, Eq)]
struct ResultScore {
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
    fn new(track: &types::Track, result: &rspotify::model::FullTrack) -> Self {
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

    fn score(&self) -> u32 {
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

pub(crate) async fn search(client: &Client, track: &types::Track) {
    let query = format!("{} artist:{}", track.title, track.artist.name);

    let result = client
        .search(
            &query,
            SearchType::Track,
            Some(MARKET),
            Some(IncludeExternal::Audio),
            Some(10),
            None,
        )
        .await;

    let Ok(SearchResult::Tracks(tracks)) = result else {
        return;
    };

    if tracks.items.is_empty() {
        return;
    }

    dbg!(track);
    println!(
        "Search track: {}, artist: {}, album: {}, # {}",
        track.title, track.artist.name, track.album.title, track.number
    );

    let mut tm = TrackMatcher::new(track);

    let mut best = &tracks.items[0];
    let mut best_score = ResultScore::new(track, best);

    for result in tracks.items.iter() {
        let tscore = tm.score(result);
        dbg!(tscore);

        let score = ResultScore::new(track, result);
        if score.score() > best_score.score() {
            best = result;
            best_score = score;
        }
    }

    println!(
        "Result track: {}, artist: {}, album: {}, # {}",
        best.name, best.artists[0].name, best.album.name, best.track_number
    );

    dbg!(best_score.score());
}
