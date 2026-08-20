use fuzzt::algorithms::{jaro, normalized_levenshtein};
use std::collections::BTreeSet;
use std::marker::PhantomData;
use unicode_normalization::UnicodeNormalization;

use crate::{
    db::{full, model},
    types::{Duration, SpotifyTrack},
};

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
        ];

        for (find, repl) in REPLACE {
            if find.contains(&c) {
                return *repl;
            }
        }

        c
    }

    fn remove_parens_and_brackets(c: &char) -> bool {
        const REMOVE: &[char] = &['(', ')', '[', ']'];
        !REMOVE.contains(c)
    }

    fn strip_suffix(s: &str) -> String {
        const STRIP_SUFFIXES: &[char] = &['!', '.', '?'];
        s.trim_end_matches(STRIP_SUFFIXES).to_string()
    }

    fn drop_separators(s: &&str) -> bool {
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

    fn strip_combining_diacritical_marks(c: &char) -> bool {
        !matches!(*c, '\u{0300}'..='\u{036F}')
    }

    let input = s;

    let normalized = s
        .nfd()
        .filter(strip_combining_diacritical_marks)
        .nfc()
        .map(|c| c.to_ascii_lowercase())
        .map(replace_equivalent_char)
        .filter(remove_parens_and_brackets)
        .collect::<String>()
        .split(is_segment_separator)
        .filter_map(trim_empty_segments)
        .filter(drop_separators)
        .map(replace_segment)
        .collect::<Vec<&str>>()
        .join(" ");

    let normalized = strip_suffix(&normalized);

    // did normalization produce a radically different value from the input?
    let diff = (1.0 - jaro(input, &normalized)) * 100.0;
    if diff > 50.0 {
        tracing::debug!(
            input,
            normalized,
            diff,
            "potentially over-normalized search string"
        );
    }

    normalized
}

fn common_words(a: &str, b: &str) -> f64 {
    const MIN_SHARED_WORDS: usize = 2;

    fn join<'a, T>(words: T) -> String
    where
        T: IntoIterator<Item = &'a &'a str>,
    {
        words.into_iter().copied().collect::<Vec<&str>>().join(" ")
    }

    let set_a: BTreeSet<&str> = a.split_whitespace().collect();
    let set_b: BTreeSet<&str> = b.split_whitespace().collect();

    let shared: Vec<_> = set_a.intersection(&set_b).collect();
    let shared_count = shared.len();

    let shared = join(shared);
    let a_only = join(set_a.difference(&set_b));
    let b_only = join(set_b.difference(&set_a));

    let with_a = format!("{shared} {a_only}").trim().to_string();
    let with_b = format!("{shared} {b_only}").trim().to_string();

    if shared_count >= MIN_SHARED_WORDS {
        normalized_levenshtein(&with_a, &with_b)
            .max(normalized_levenshtein(&shared, &with_a))
            .max(normalized_levenshtein(&shared, &with_b))
    } else {
        normalized_levenshtein(&with_a, &with_b)
    }
}

const CREDIT_MARKERS: &[&str] = &[
    "feat",
    "feat.",
    "feat:",
    "featuring",
    "ft",
    "ft.",
    "ft:",
    "w/",
    "with",
];

fn contains(needle: &str, haystack: &[&str]) -> bool {
    haystack.iter().any(|s| s.eq_ignore_ascii_case(needle))
}

const TAGS: [&str; 8] = [
    "acoustic",
    "demo",
    "dub",
    "instrumental",
    "live",
    "remix",
    "reprise",
    "rework",
];

trait Tags {
    fn set_tags(self, tags: &mut usize);
}

impl Tags for &str {
    fn set_tags(self, tags: &mut usize) {
        for elem in self.split(|c: char| !c.is_alphanumeric()) {
            for (i, tag) in TAGS.iter().enumerate() {
                if tag.eq_ignore_ascii_case(elem) {
                    *tags |= 1 << i;
                    break;
                }
            }
        }
    }
}

// A track title and its parsed metadata. There are a couple things that we may
// encounter within a track title, and this aims to detect and parse them:
//
// * featured artist: Bandcamp puts featured artists in the track title, but
//     Spotify usually does not.
// * release tags: "live", "demo", etc
#[derive(Debug, Clone)]
pub(crate) struct TrackTitle {
    original: String,
    core: String,
    featured: Vec<String>,
    tags: usize,
}

impl<T: AsRef<str>> From<T> for TrackTitle {
    fn from(value: T) -> Self {
        Self::from_str(value.as_ref())
    }
}

impl TrackTitle {
    pub(crate) fn from_str(raw: &str) -> Self {
        let mut core = String::new();
        let mut featured = Vec::new();
        let mut tags = 0usize;

        {
            let mut slice = raw;
            while let Some((from, last)) = slice.char_indices().find_map(|(i, c)| match c {
                '(' => Some((i, ')')),
                '[' => Some((i, ']')),
                _ => None,
            }) {
                core.push_str(&slice[..from]);
                slice = &slice[(from + 1)..];

                let Some(to) = slice.find(last) else {
                    break;
                };

                let group = &slice[..to];
                if group.is_empty() {
                    break;
                }
                slice = &slice[(to + 1)..];

                match group.split_once(|c: char| c.is_whitespace()) {
                    Some((first, rest)) if contains(first, CREDIT_MARKERS) => {
                        featured.extend(TrackArtists::split(rest));
                    }
                    _ => {
                        group.set_tags(&mut tags);
                    }
                }
            }

            core.push_str(slice);
        }

        // extract a possible credited artist name from the title
        {
            let words: Vec<&str> = core.split_whitespace().collect();
            let found = words
                .iter()
                // if we see `with` outside of brackets/parens, it's more likely
                // to be part of the title and not signifying a featured artist
                .position(|word| {
                    !word.eq_ignore_ascii_case("with") && contains(word, CREDIT_MARKERS)
                });

            if let Some(found) = found
                && found > 0
                && found < (words.len() - 1)
            {
                let head = &words[..found];
                let tail = &words[found + 1..];

                let credited = tail.join(" ");
                featured.extend(TrackArtists::split(&credited));

                core = head.join(" ");
            }
        };

        let core = core
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end_matches([' ', '-', ',', ':', '/'])
            .trim()
            .to_string();

        if let Some((_, trailing)) = core.rsplit_once(" - ") {
            trailing.set_tags(&mut tags);
        }

        Self {
            tags,
            core: if core.is_empty() {
                raw.trim().to_string()
            } else {
                core
            },
            original: raw.to_string(),
            featured,
        }
    }

    pub(crate) fn core(&self) -> &str {
        &self.core
    }
}

// When a track has multiple artists on it, Bandcamp embeds them into a single
// string, whereas Spotify records them as separate, structured array elements
// in the track metadata.
#[derive(Debug, Clone)]
pub(crate) struct TrackArtists {
    // for Bandcamp, the "original", unmodified artist string
    // for Spotify, the artist names joined together
    joined: String,

    // individual artist names, with the lead/main artist first
    split: Vec<String>,
}

impl<T: AsRef<str>> From<T> for TrackArtists {
    fn from(value: T) -> Self {
        Self::from_str(value.as_ref())
    }
}

impl TrackArtists {
    pub(crate) fn split(raw: &str) -> Vec<String> {
        // normalize `w/` to `&` so we can split on `/`
        let norm = raw.replace(" w/ ", " & ");

        let mut names = Vec::new();
        let mut cur: Vec<&str> = Vec::new();

        for chunk in norm.split(['/', ',', '&', ';']) {
            cur.clear();

            for word in chunk.split_whitespace() {
                if contains(word, CREDIT_MARKERS) || contains(word, &["and", "vs", "vs."]) {
                    if !cur.is_empty() {
                        names.push(cur.join(" "));
                        cur.clear();
                    }
                } else {
                    cur.push(word);
                }
            }

            if !cur.is_empty() {
                names.push(cur.join(" "));
            }
        }

        names
    }

    pub(crate) fn from_str(raw: &str) -> Self {
        Self {
            joined: raw.trim().to_string(),
            split: Self::split(raw),
        }
    }

    pub(crate) fn from_iter<'a>(names: impl Iterator<Item = &'a str>) -> Self {
        let split: Vec<String> = names
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(String::from)
            .collect();

        Self {
            joined: split.join(" & "),
            split,
        }
    }

    fn extend(mut self, extra: &[String]) -> Self {
        if !extra.is_empty() {
            self.split.extend(extra.iter().map(|s| s.to_owned()));
        }
        self
    }

    pub(crate) fn primary(&self) -> &str {
        self.split.first().map(String::as_str).unwrap_or_default()
    }

    // returns several permutations of the track's artist(s) for partial string
    // comparison:
    //
    // * the joined/original artist string
    // * each artist on their own
    // * a sorted version of the joined artist string
    fn permutations(&self) -> BTreeSet<String> {
        let names: BTreeSet<&str> = self
            .split
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .collect();

        let mut elems: BTreeSet<String> = Default::default();
        for name in &names {
            elems.insert(name.to_string());
        }

        let mut elems: BTreeSet<String> = names.iter().copied().map(String::from).collect();

        if names.len() > 1 {
            elems.insert(names.iter().copied().collect::<Vec<_>>().join(" & "));
        }

        if !self.joined.trim().is_empty() {
            elems.insert(self.joined.clone());
        }

        elems
    }
}

pub(crate) trait MatchType {
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
impl MatchType for model::Release {}
impl MatchType for model::Artist {}

#[derive(Debug)]
struct StringMatcher<MT: MatchType> {
    original: String,
    normalized: String,
    _mt: PhantomData<MT>,
}

impl<MT: MatchType> StringMatcher<MT> {
    fn new<T: Into<String>>(original: T) -> Self {
        let original = original.into();
        let normalized = normalize(&original);

        Self {
            original,
            normalized,
            _mt: Default::default(),
        }
    }

    fn score(&self, s: &str) -> f64 {
        let norm = normalize(s);

        if self.original.eq_ignore_ascii_case(s) {
            tracing::trace!(
                kind = MT::label(),
                subject = %self.original,
                candidate = s,
                "exact match",
            );
            return 100.0;
        }

        let jaro_score = jaro(&self.normalized, &norm) * 100.0;
        let word_score = common_words(&self.normalized, &norm) * 100.0;
        let score = jaro_score.max(word_score);

        tracing::trace!(
            kind = MT::label(),
            subject = %self.normalized,
            candidate = %norm,
            jaro_score,
            word_score,
            score,
            "compared",
        );

        score
    }
}

mod matcher {
    use super::*;

    pub type Release = StringMatcher<model::Release>;

    #[derive(Debug)]
    pub struct Artist {
        permutations: BTreeSet<String>,
        matchers: Vec<StringMatcher<model::Artist>>,
    }

    impl Artist {
        pub fn new(artists: TrackArtists) -> Self {
            let permutations = artists.permutations();
            let mut matchers = Vec::with_capacity(permutations.len());
            for subject in &permutations {
                matchers.push(StringMatcher::new(subject));
            }

            Self {
                permutations,
                matchers,
            }
        }

        pub fn score(&self, other: &TrackArtists) -> (f64, String) {
            let permutations = other.permutations();

            let mut best = 0f64;
            let mut matched = permutations
                .iter()
                .nth(0)
                .expect("TrackArtists.permutations() never returns an empty data set");

            for matcher in &self.matchers {
                for candidate in &permutations {
                    let score = matcher.score(candidate);
                    if score > best {
                        best = score;
                        matched = candidate;
                    }
                }
            }

            tracing::trace!(
                subjects = ?self.permutations,
                candidates = ?permutations,
                best = ?matched,
                score = best,
                "compared artist forms",
            );

            (best, matched.to_owned())
        }
    }

    #[derive(Debug)]
    pub struct Title {
        original: StringMatcher<TrackTitle>,
        core: StringMatcher<TrackTitle>,
        tags: usize,
    }

    impl Title {
        pub fn new(title: TrackTitle) -> Self {
            let TrackTitle {
                original,
                core,
                featured: _,
                tags,
            } = title;

            Self {
                original: StringMatcher::new(original),
                core: StringMatcher::new(core),
                tags,
            }
        }

        pub fn score(&self, other: &TrackTitle) -> f64 {
            let core = self.core.score(other.core());
            let original = self.original.score(&other.original);
            core.max(original)
        }

        pub fn same_version(&self, other: &TrackTitle) -> bool {
            self.tags == other.tags
        }
    }

    pub(crate) trait MatchDuration {
        fn track_max_diff(&self) -> Duration;
        fn track_score(&self, other: Duration) -> Option<f64>;
    }

    impl MatchDuration for Duration {
        fn track_max_diff(&self) -> Duration {
            const LIMIT: Duration = Duration::from_secs(15);
            const RATIO: f64 = 0.05;

            LIMIT.max(self.mul_f64(RATIO))
        }

        fn track_score(&self, other: Duration) -> Option<f64> {
            const EXACT: Duration = Duration::from_secs(2);

            let diff = self.abs_diff(other);
            let max = self.track_max_diff();

            if diff <= EXACT {
                return Some(100.0);
            } else if diff >= max {
                return None;
            }

            let falloff = (diff - EXACT).as_secs_f64() / (max - EXACT).as_secs_f64();

            Some((1.0 - falloff) * 100.0)
        }
    }

    pub(crate) trait MatchTrackNumber {
        fn track_score(self, other: usize) -> f64;
    }

    impl MatchTrackNumber for usize {
        fn track_score(self, other: usize) -> f64 {
            if self == other { 100.0 } else { 0.0 }
        }
    }
}

#[derive(Debug)]
struct MatchParams {
    title: TrackTitle,
    artists: TrackArtists,
    album: String,
    number: usize,
    duration: Duration,
}

impl MatchParams {
    fn new<T, A, R>(title: T, artists: A, release: R, number: usize, duration: Duration) -> Self
    where
        T: Into<TrackTitle>,
        A: Into<TrackArtists>,
        R: ToString,
    {
        let title = title.into();

        let artists = artists.into().extend(&title.featured);

        Self {
            title,
            artists,
            album: release.to_string(),
            number,
            duration,
        }
    }
}

impl From<&SpotifyTrack> for MatchParams {
    fn from(value: &SpotifyTrack) -> MatchParams {
        let artists = TrackArtists::from_iter(value.artists.iter().map(|a| a.name.as_str()));
        Self::new(
            &value.name,
            artists,
            &value.album.name,
            value.track_number as usize,
            value.duration.to_std().unwrap_or_default(),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MatchResult {
    Match(f64),
    WrongVersion,
    WrongDuration(Duration),
    WrongTitle(f64),
    WrongArtist(f64),
}

impl MatchResult {
    pub(crate) fn matched(&self) -> Option<f64> {
        match self {
            Self::Match(score) => Some(*score),
            Self::WrongVersion
            | Self::WrongDuration(_)
            | Self::WrongTitle(_)
            | Self::WrongArtist(_) => None,
        }
    }

    pub(crate) fn score(&self) -> f64 {
        match self {
            Self::Match(score) | Self::WrongTitle(score) | Self::WrongArtist(score) => *score,
            Self::WrongVersion | Self::WrongDuration(_) => 0.0,
        }
    }
}

impl std::fmt::Display for MatchResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Match(score) => write!(f, "matched at {score:.1}"),
            Self::WrongVersion => write!(f, "a different version"),
            Self::WrongDuration(diff) => write!(f, "duration differs by {diff:?}"),
            Self::WrongTitle(score) => write!(f, "title scored {score:.1}"),
            Self::WrongArtist(score) => write!(f, "artist scored {score:.1}"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct TrackMatcher {
    title: matcher::Title,
    artists: matcher::Artist,
    album: matcher::Release,
    number: usize,
    duration: Duration,
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
    pub(crate) fn new(
        title: &str,
        artist: &str,
        credited_artist: Option<&str>,
        album: &str,
        number: usize,
        duration: Duration,
    ) -> Self {
        let title = TrackTitle::from_str(title);

        let mut credited: Vec<String> =
            credited_artist.map(TrackArtists::split).unwrap_or_default();
        credited.extend(title.featured.iter().cloned());

        let artists = TrackArtists::from_str(artist).extend(&credited);

        Self {
            title: matcher::Title::new(title),
            artists: matcher::Artist::new(artists),
            album: matcher::Release::new(album),
            number,
            duration,
        }
    }

    pub(crate) fn from_track(track: &full::Track) -> TrackMatcher {
        Self::new(
            &track.track.title,
            &track.artist.name,
            track.track.credited_artist.as_deref(),
            &track.release.title,
            track.track.release_track_number as usize,
            Duration::from_secs_f64(track.track.duration),
        )
    }

    pub(crate) fn score(&self, spotify_track: &SpotifyTrack) -> MatchResult {
        let span = tracing::debug_span!(
            "candidate",
            id = spotify_track
                .id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_default(),
        );
        let _guard = span.enter();

        self.score_params(MatchParams::from(spotify_track))
    }

    fn score_params(&self, params: MatchParams) -> MatchResult {
        use matcher::*;

        let title = self.title.score(&params.title);
        let (artist, matched_artist) = self.artists.score(&params.artists);
        let album = self.album.score(&params.album);
        let tracknum = self.number.track_score(params.number);
        let duration = self.duration.track_score(params.duration);

        let weighted = (title * TITLE_WEIGHT)
            + (artist * ARTIST_WEIGHT)
            + (album * ALBUM_WEIGHT)
            + (tracknum * TRACKNUM_WEIGHT)
            + (duration.unwrap_or_default() * DURATION_WEIGHT);

        let composite = (weighted / self.max_possible() as f64) * 100.0;

        let result = if !self.title.same_version(&params.title) {
            MatchResult::WrongVersion
        } else if duration.is_none() {
            MatchResult::WrongDuration(self.duration.abs_diff(params.duration))
        } else if title < MIN_TITLE_SCORE {
            MatchResult::WrongTitle(title)
        } else if artist < MIN_ARTIST_SCORE {
            MatchResult::WrongArtist(artist)
        } else {
            MatchResult::Match(composite)
        };

        fn round(score: f64) -> f64 {
            (score * 10.0).round() / 10.0
        }

        tracing::debug!(
            cand.title = %params.title.original,
            cand.core = %params.title.core(),
            cand.artists = %params.artists.joined,
            cand.album = %params.album,
            cand.number = params.number,
            cand.secs = params.duration.as_secs_f64(),
            cand.tags = ?params.title.tags,
            score.title = round(title),
            score.artist = round(artist),
            score.matched_artist = %matched_artist,
            score.album = round(album),
            score.number = round(tracknum),
            score.duration = round(duration.unwrap_or_default()),
            score.composite = round(composite),
            min_score.title = MIN_TITLE_SCORE,
            min_score.artist = MIN_ARTIST_SCORE,
            max_diff.duration = ?self.duration.track_max_diff(),
            "{result}",
        );

        result
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
    use super::matcher::*;
    use super::*;

    impl From<(&str, &str, &str)> for TrackMatcher {
        fn from(value: (&str, &str, &str)) -> Self {
            Self::new(value.0, value.1, None, value.2, 1, Duration::from_secs(60))
        }
    }

    impl From<(&str, &str, &str, i32, Duration)> for TrackMatcher {
        fn from(value: (&str, &str, &str, i32, Duration)) -> Self {
            Self::new(value.0, value.1, None, value.2, value.3 as usize, value.4)
        }
    }

    impl<A> From<(&str, A, &str)> for MatchParams
    where
        A: Into<TrackArtists>,
    {
        fn from(value: (&str, A, &str)) -> Self {
            Self::new(value.0, value.1, value.2, 1, Duration::from_secs(60))
        }
    }

    impl<A> From<(&str, A, &str, i32, Duration)> for MatchParams
    where
        A: Into<TrackArtists>,
    {
        fn from(value: (&str, A, &str, i32, Duration)) -> Self {
            Self::new(value.0, value.1, value.2, value.3 as usize, value.4)
        }
    }

    fn tags_to_vec(tags: usize) -> Vec<&'static str> {
        TAGS.iter()
            .enumerate()
            .filter_map(|(i, t)| {
                if tags & (1 << i) == (1 << i) {
                    Some(*t)
                } else {
                    None
                }
            })
            .collect()
    }

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
            let matcher: StringMatcher<TrackTitle> = StringMatcher::new(bandcamp_title);
            let result = matcher.score(search_result);
            assert!(
                result > 0.0,
                "search for '{search_result}' in '{bandcamp_title}' yielded a zero score"
            );
        }
    }

    #[test]
    fn normalize_folds_diacritics() {
        assert_eq!("desiree", normalize("Desirée"));
        assert_eq!("brujula", normalize("Brújula"));
        assert_eq!("ceska reka", normalize("Česká Řeka"));
        assert_eq!("君が目", normalize("君が目"));
    }

    #[test]
    fn normalize_short_titles() {
        assert_eq!("wake", normalize("Wake."));
        assert_eq!("head", normalize("head!"));
        assert_eq!("001", normalize("001."));
    }

    #[test]
    fn common_word_scores() {
        assert_eq!(
            1.0,
            common_words("small white animal", "small white animal 2026 remaster")
        );

        assert!(common_words("title", "nope nope bad title") < 0.5);
        assert!(common_words("arise", "arise to the sun") < 0.5);
    }

    #[test]
    fn title_parse() {
        let cases = vec![
            // (raw, core, featured)
            ("Simple (feat. Sene)", "Simple", vec!["Sene"]),
            ("Bilali feat. Samir Langus", "Bilali", vec!["Samir Langus"]),
            ("Knew It All (Ft: Oddisee)", "Knew It All", vec!["Oddisee"]),
            (
                "Lifelike ft. Moor Mother & 700 Bliss",
                "Lifelike",
                vec!["Moor Mother", "700 Bliss"],
            ),
            (
                "Stet Dreams Come True featuring Calvin III",
                "Stet Dreams Come True",
                vec!["Calvin III"],
            ),
            ("Quartet (2022)", "Quartet", vec![]),
            ("Takamaru (Eurlica Remix)", "Takamaru", vec![]),
            (
                "You Can't Negotiate With Zombies [Debut Album]",
                "You Can't Negotiate With Zombies",
                vec![],
            ),
            // unclosed group
            ("Wake (up", "Wake up", vec![]),
            // title that _only_ contains a tag
            ("(Instrumental)", "(Instrumental)", vec![]),
        ];

        for (raw, core, featured) in cases {
            let title = TrackTitle::from_str(raw);
            assert_eq!(core, title.core(), "'{raw}' .core");
            assert_eq!(featured, title.featured, "'{raw}' .featured");
        }
    }

    #[test]
    fn title_version_tags() {
        let cases = vec![
            (
                "Alone + Easy Target (Live from Somewhere 2025)",
                vec!["live"],
            ),
            ("Takamaru (Eurlica Remix)", vec!["remix"]),
            ("What's His Name (Instrumental)", vec!["instrumental"]),
            // we don't consider `remaster` a different version
            ("Small White Animal - 2026 Remaster", vec![]),
            ("Quartet (2022)", vec![]),
            ("Alone + Easy Target", vec![]),
        ];

        for (raw, tags) in cases {
            assert_eq!(
                tags,
                tags_to_vec(TrackTitle::from_str(raw).tags),
                "'{raw}' .tags"
            );
        }
    }

    #[test]
    fn artists_parse() {
        let cases = vec![
            (
                "Roser/Moser/Asselbergs",
                vec!["Roser", "Moser", "Asselbergs"],
            ),
            (
                "Rodolphe Burger, Erik Marchand",
                vec!["Rodolphe Burger", "Erik Marchand"],
            ),
            (
                "Von Pea & The Other Guys",
                vec!["Von Pea", "The Other Guys"],
            ),
            (
                "Duncecap feat. Samurai Banana",
                vec!["Duncecap", "Samurai Banana"],
            ),
            ("Ana w/ Bea", vec!["Ana", "Bea"]),
            ("Amulets", vec!["Amulets"]),
        ];

        for (raw, split) in cases {
            let artists = TrackArtists::from_str(raw);
            assert_eq!(split, artists.split, "split of '{raw}'");
            assert_eq!(split[0], artists.primary(), "lead of '{raw}'");
        }
    }

    #[test]
    fn track_matcher_packed_artist() {
        let track = (
            "Knew It All (Ft: Oddisee)",
            "Von Pea & The Other Guys",
            "album",
            7,
            Duration::from_secs(156),
        );

        let other = (
            "Knew It All (feat. Oddisee)",
            TrackArtists::from_iter(["Von Pea", "The Other Guys", "Oddisee"].into_iter()),
            "album",
            7,
            Duration::from_secs(156),
        );

        let matcher = TrackMatcher::from(track);

        assert_eq!(
            MatchResult::Match(100.0),
            matcher.score_params(other.into())
        );
    }

    #[test]
    fn track_matcher_rejects_other_version() {
        let track = (
            "Alone (Live in Berlin)",
            "artist",
            "album",
            4,
            Duration::from_secs(240),
        );

        let other = ("Alone", "artist", "album", 4, Duration::from_secs(240));

        let matcher = TrackMatcher::from(track);

        assert_eq!(
            MatchResult::WrongVersion,
            matcher.score_params(other.into())
        );
    }

    #[test]
    fn track_matcher_exact() {
        let track = ("track", "artist", "album", 2, Duration::from_secs(30));

        let matcher = TrackMatcher::from(track);

        let score = matcher.score_params(track.into());

        assert_eq!(MatchResult::Match(100.0), score);
    }

    #[test]
    fn track_matcher_similar_title() {
        let track = ("my track name!!", "artist", "album");

        let alt = format!("{}!", track.0);
        let other = (alt.as_str(), "artist", "album");

        let matcher = TrackMatcher::from(track);

        let score = matcher.score_params(other.into());
        assert_eq!(MatchResult::Match(100.0), score);
    }

    #[test]
    fn track_matcher_wrong_title() {
        let track = ("title", "artist", "album");

        let params = MatchParams::from(("nope nope bad title", "artist", "album"));

        let matcher = TrackMatcher::from(track);

        let score = matcher.score_params(params);

        assert!(
            matches!(score, MatchResult::WrongTitle(_)),
            "expected a title rejection, got {score:?}"
        );
    }

    #[test]
    fn duration_match() {
        let matcher = Duration::from_secs(180);

        for secs in [178, 179, 180, 181, 182] {
            assert_eq!(
                Some(100.0),
                matcher.track_score(Duration::from_secs(secs)),
                "{secs}s should be indistinguishable from 180s"
            );
        }

        for secs in [0, 90, 195, 240, 360] {
            assert_eq!(
                None,
                matcher.track_score(Duration::from_secs(secs)),
                "{secs}s should be disqualified"
            );
        }

        let near = matcher
            .track_score(Duration::from_secs(185))
            .expect("185s is inside the limit");
        let far = matcher
            .track_score(Duration::from_secs(192))
            .expect("192s is inside the limit");

        assert!(near > far, "185s ({near}) should outrank 192s ({far})");
    }

    #[test]
    fn duration_matcher_scales_with_length() {
        let short = Duration::from_secs(90);
        assert_eq!(Duration::from_secs(15), short.track_max_diff());

        let long = Duration::from_secs(1200);
        assert_eq!(Duration::from_secs(60), long.track_max_diff());

        assert!(
            long.track_score(Duration::from_secs(1250)).is_some(),
            "50s off a 20 minute track is within 5%"
        );
        assert_eq!(
            None,
            long.track_score(Duration::from_secs(1270)),
            "70s off a 20 minute track is not"
        );
    }

    #[test]
    fn track_matcher_wrong_duration() {
        let track = ("title", "artist", "album", 2, Duration::from_secs(180));

        let other = ("title", "artist", "album", 2, Duration::from_secs(240));

        let matcher = TrackMatcher::from(track);

        let score = matcher.score_params(other.into());

        assert!(
            matches!(score, MatchResult::WrongDuration(_)),
            "expected a duration rejection, got {score:?}"
        );
    }

    #[test]
    fn track_matcher_wrong_artist() {
        let track = ("title", "artist", "album", 2, Duration::from_secs(30));

        let other = (
            "title",
            "nope not the right artist",
            "album",
            2,
            Duration::from_secs(30),
        );

        let matcher = TrackMatcher::from(track);

        let score = matcher.score_params(other.into());

        assert!(
            matches!(score, MatchResult::WrongArtist(_)),
            "expected an artist rejection, got {score:?}"
        );
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
            let track = ("title", case.0, "album", 2, Duration::from_secs(30));

            let other = (
                track.0,
                TrackArtists::from_iter(case.1.iter().copied()),
                track.2,
                track.3,
                track.4,
            );

            let matcher = TrackMatcher::from(track);
            let score = matcher.score_params(other.into());

            assert_eq!(
                MatchResult::Match(100.0),
                score,
                "track: '{}', result: '{:?}'",
                case.0,
                case.1
            );
        }
    }
}
