use std::marker::PhantomData;

use std::collections::BTreeSet;

use fuzzt::algorithms::{jaro, normalized_levenshtein};
use unicode_normalization::UnicodeNormalization;

use crate::types::{self, Duration, SpotifyTrack};

const TITLE_WEIGHT: f64 = 100.0;
const ARTIST_WEIGHT: f64 = 50.0;
const ALBUM_WEIGHT: f64 = 10.0;
const DURATION_WEIGHT: f64 = 5.0;
const TRACKNUM_WEIGHT: f64 = 5.0;

const MIN_TITLE_SCORE: f64 = 95.0;
const MIN_ARTIST_SCORE: f64 = 90.0;

const DURATION_MATCH: Duration = Duration::from_secs(2);

/// Floor for the duration gate, and the fraction of the track it widens to.
/// A flat 15s is 6% of a four-minute song but 1% of a twenty-minute one, and
/// long-form pieces routinely differ by more than that between a bandcamp
/// master and a Spotify release. Measured over same-artist wrong pairs, moving
/// from a flat 15s to 5% lets 13.7% through instead of 13.1%; at 10% it is
/// 19.2%, which is where the gate stops earning its keep.
const DURATION_LIMIT: Duration = Duration::from_secs(15);
const DURATION_LIMIT_RATIO: f64 = 0.05;

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

    fn keep_char(c: &char) -> bool {
        const REMOVE: &[char] = &['(', ')', '[', ']'];
        !REMOVE.contains(c)
    }

    fn strip_suffix(s: &str) -> String {
        const STRIP_SUFFIXES: &[char] = &['!', '.', '?'];
        s.trim_end_matches(STRIP_SUFFIXES).to_string()
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

    // Strip the Combining Diacritical Marks block only. `Mn` at large would take
    // the Japanese dakuten with it, turning が into か.
    let s = s
        .nfd()
        .filter(|c| !matches!(c, '\u{0300}'..='\u{036F}'))
        .nfc()
        .collect::<String>()
        .to_lowercase();

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
    // not a warning: a title that is mostly punctuation is odd, not wrong.
    let diff = (1.0 - jaro(input, &normalized)) * 100.0;
    if diff > 50.0 {
        tracing::debug!(input, normalized, diff, "normalization was destructive");
    }

    normalized
}

/// A subset comparison scores a flat 1.0, so it is only trustworthy once enough
/// words are shared. Below this a one-word title matches every longer title
/// containing that word: `title` would score 100 against `nope nope bad title`.
const MIN_SHARED_TOKENS: usize = 2;

/// Set comparison that makes whole-token insertion free. Because the sets are
/// sorted, the shared tokens are a literal prefix of both sides, so when one
/// title's tokens are a subset of the other's the comparison is exactly 1.0 --
/// which is the shape of a featured artist, a section prefix, and a
/// parenthetical tag. Jaro cannot see through any of them: an inserted word
/// blows out both of its length-ratio terms at once.
fn token_set(a: &str, b: &str) -> f64 {
    let ta: BTreeSet<&str> = a.split_whitespace().collect();
    let tb: BTreeSet<&str> = b.split_whitespace().collect();

    let join = |tokens: Vec<&&str>| tokens.into_iter().copied().collect::<Vec<&str>>().join(" ");

    let shared = join(ta.intersection(&tb).collect());
    let a_only = join(ta.difference(&tb).collect());
    let b_only = join(tb.difference(&ta).collect());

    let with_a = format!("{shared} {a_only}").trim().to_string();
    let with_b = format!("{shared} {b_only}").trim().to_string();

    let subset = if ta.intersection(&tb).count() >= MIN_SHARED_TOKENS {
        normalized_levenshtein(&shared, &with_a).max(normalized_levenshtein(&shared, &with_b))
    } else {
        0.0
    };

    // comparing the two sorted joins is what makes this symmetric: it does not
    // matter which platform carries the extra tokens
    subset.max(normalized_levenshtein(&with_a, &with_b))
}

/// Words that introduce a performer credit rather than part of a title.
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

/// `with` is omitted: outside brackets it is far more often part of the title
/// than a credit.
const BARE_CREDIT_MARKERS: &[&str] = &[
    "feat",
    "feat.",
    "feat:",
    "featuring",
    "ft",
    "ft.",
    "ft:",
    "w/",
];

/// Words separating one performer from the next within a single field.
const ARTIST_SEPARATORS: &[&str] = &["and", "vs", "vs."];

/// Tags naming a different performance rather than a different master. A live
/// take and the studio recording share a title and often a runtime, so pulling
/// the tag out of the title makes them indistinguishable unless it also vetoes.
/// `remaster`, `deluxe` and friends are deliberately absent: same performance.
const VERSION_TAGS: &[&str] = &[
    "live",
    "remix",
    "instrumental",
    "demo",
    "acoustic",
    "reprise",
    "dub",
    "rework",
];

fn version_tags(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();

    let mut tags: Vec<String> = VERSION_TAGS
        .iter()
        .filter(|tag| words.contains(&&***tag))
        .map(|tag| tag.to_string())
        .collect();

    tags.sort();
    tags.dedup();
    tags
}

fn is_marker(word: &str, markers: &[&str]) -> bool {
    let lower = word.to_lowercase();
    markers.contains(&lower.as_str())
}

/// The performers a bracketed group names, or nothing when the group is a tag
/// (`Remastered`, `Live`, a label code) rather than a credit.
fn credited(group: &str) -> Vec<String> {
    let mut words = group.split_whitespace();
    let Some(first) = words.next() else {
        return vec![];
    };

    if !is_marker(first, CREDIT_MARKERS) {
        return vec![];
    }

    Artists::parse(&words.collect::<Vec<_>>().join(" ")).split
}

/// Split at the first credit marker, into the part before it and the part after.
fn split_at_marker(s: &str, markers: &[&str]) -> Option<(String, String)> {
    let words: Vec<&str> = s.split_whitespace().collect();
    let pos = words.iter().position(|w| is_marker(w, markers))?;

    let head = words[..pos].join(" ");
    let tail = words[pos + 1..].join(" ");

    if head.is_empty() || tail.is_empty() {
        return None;
    }

    Some((head, tail))
}

fn tidy(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches([' ', '-', ',', ':', '/'])
        .trim()
        .to_string()
}

/// A track title with the credits packed into it pulled out. Bandcamp writes
/// `Simple (feat. Sene)` for a track Spotify indexes as `Simple` with Sene as a
/// performer, so neither the query nor the comparison should carry the
/// parenthetical.
#[derive(Debug, Clone)]
pub(crate) struct Title {
    original: String,
    core: String,
    featured: Vec<String>,
    /// version tags found in the parts stripped from `core`, sorted
    tags: Vec<String>,
}

impl Title {
    pub(crate) fn parse(raw: &str) -> Self {
        let mut core = String::new();
        let mut group = String::new();
        let mut featured = Vec::new();
        let mut tag_text = String::new();
        let mut depth = 0usize;

        for c in raw.chars() {
            match c {
                '(' | '[' => {
                    depth += 1;
                    if depth == 1 {
                        group.clear();
                        continue;
                    }
                }
                ')' | ']' if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        let credits = credited(&group);
                        if credits.is_empty() {
                            tag_text.push(' ');
                            tag_text.push_str(&group);
                        } else {
                            featured.extend(credits);
                        }
                        continue;
                    }
                }
                _ => {}
            }

            if depth == 0 {
                core.push(c);
            } else {
                group.push(c);
            }
        }

        // an unclosed bracket is punctuation, not structure
        if depth > 0 {
            core.push_str(&group);
        }

        if let Some((head, credit)) = split_at_marker(&core, BARE_CREDIT_MARKERS) {
            featured.extend(Artists::parse(&credit).split);
            core = head;
        }

        let core = tidy(&core);

        // a trailing dash segment is where Spotify puts what bandcamp brackets
        if let Some((_, trailing)) = core.rsplit_once(" - ") {
            tag_text.push(' ');
            tag_text.push_str(trailing);
        }

        Self {
            tags: version_tags(&tag_text),
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

    /// On compilations bandcamp leaves the artist field as `Various Artists` and
    /// writes the real performer into the title: `Circadian Rhythm - Shela`.
    pub(crate) fn credited_artist(&self) -> Option<(String, String)> {
        for sep in [" - ", " : ", " -- "] {
            if let Some((artist, title)) = self.core.split_once(sep) {
                let (artist, title) = (artist.trim(), title.trim());
                if !artist.is_empty() && !title.is_empty() {
                    return Some((artist.to_string(), title.to_string()));
                }
            }
        }

        None
    }
}

/// The performers named in an artist field. Bandcamp packs them into one string
/// (`Roser/Moser/Asselbergs`); Spotify splits them into an array.
#[derive(Debug, Clone)]
pub(crate) struct Artists {
    /// the field as written, or Spotify's array joined back together
    packed: String,
    /// individual performers, lead first; never empty
    split: Vec<String>,
}

impl Artists {
    pub(crate) fn parse(raw: &str) -> Self {
        // `w/` would otherwise be torn in half by the `/` separator
        let separated = raw.replace(" w/ ", " & ");

        let mut split = Vec::new();
        for chunk in separated.split(['/', ',', '&', ';']) {
            let mut cur: Vec<&str> = Vec::new();

            for word in chunk.split_whitespace() {
                if is_marker(word, CREDIT_MARKERS) || is_marker(word, ARTIST_SEPARATORS) {
                    if !cur.is_empty() {
                        split.push(cur.join(" "));
                        cur.clear();
                    }
                } else {
                    cur.push(word);
                }
            }

            if !cur.is_empty() {
                split.push(cur.join(" "));
            }
        }

        let packed = raw.trim().to_string();
        if split.is_empty() {
            split.push(packed.clone());
        }

        Self { packed, split }
    }

    fn from_list<'a>(names: impl Iterator<Item = &'a str>) -> Self {
        let split: Vec<String> = names.map(|s| s.to_string()).collect();

        if split.is_empty() {
            return Self {
                packed: String::new(),
                split: vec![String::new()],
            };
        }

        Self {
            packed: split.join(" & "),
            split,
        }
    }

    pub(crate) fn lead(&self) -> &str {
        &self.split[0]
    }

    pub(crate) fn is_various(&self) -> bool {
        let packed = self.packed.to_lowercase();
        packed.contains("various") || packed == "v/a" || packed == "va"
    }

    /// Every form worth comparing: the field as written, each performer alone,
    /// and an alphabetised join so a packed bandcamp string can line up with
    /// Spotify's array whatever order the two use.
    fn forms(&self, extra: &[String]) -> Vec<String> {
        let mut performers: Vec<String> = self
            .split
            .iter()
            .chain(extra.iter())
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .collect();

        let mut forms = performers.clone();
        forms.push(self.packed.clone());

        performers.sort();
        performers.dedup();
        forms.push(performers.join(" & "));

        forms.retain(|s| !s.trim().is_empty());
        forms.sort();
        forms.dedup();
        forms
    }
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

        if self.original.eq_ignore_ascii_case(s) {
            tracing::trace!(
                kind = MT::label(),
                subject = %self.original,
                candidate = s,
                "exact match",
            );
            return 100.0;
        }

        // the two are complementary rather than redundant: jaro covers character
        // noise inside a token, which tokenizing destroys, and token_set covers
        // whole-token insertion, which jaro cannot see. Both are logged because
        // which one carried the score is the first thing you need to know.
        let jaro = jaro(&self.normalized, &norm) * 100.0;
        let tokens = token_set(&self.normalized, &norm) * 100.0;
        let score = jaro.max(tokens);

        tracing::trace!(
            kind = MT::label(),
            subject = %self.normalized,
            candidate = %norm,
            jaro,
            token_set = tokens,
            score,
            "compared",
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
    duration: Duration,
}

impl TrackDurationMatcher {
    fn new(duration: Duration) -> Self {
        Self { duration }
    }

    /// How far a candidate may run from this track before it is a different
    /// recording. Always wider than [`DURATION_MATCH`], so the falloff below
    /// cannot divide by zero.
    fn limit(&self) -> Duration {
        DURATION_LIMIT.max(self.duration.mul_f64(DURATION_LIMIT_RATIO))
    }

    /// `None` disqualifies the candidate outright.
    fn score(&self, other: Duration) -> Option<f64> {
        let diff = self.duration.abs_diff(other);
        let limit = self.limit();

        if diff <= DURATION_MATCH {
            return Some(100.0);
        } else if diff >= limit {
            return None;
        }

        let falloff =
            (diff - DURATION_MATCH).as_secs_f64() / (limit - DURATION_MATCH).as_secs_f64();

        Some((1.0 - falloff) * 100.0)
    }
}

struct MatchParams {
    title: Title,
    artists: Artists,
    album: String,
    number: usize,
    duration: Duration,
}

impl From<&types::SpotifyTrack> for MatchParams {
    fn from(value: &types::SpotifyTrack) -> MatchParams {
        Self {
            title: Title::parse(&value.name),
            artists: Artists::from_list(value.artists.iter().map(|a| a.name.as_str())),
            album: value.album.name.clone(),
            number: value.track_number as usize,
            duration: value.duration.to_std().unwrap_or_default(),
        }
    }
}

impl From<&types::Track> for MatchParams {
    fn from(value: &types::Track) -> MatchParams {
        Self {
            title: Title::parse(&value.title),
            artists: Artists::parse(&value.artist.name),
            album: value.album.title.clone(),
            number: value.album_track_number,
            duration: value.duration,
        }
    }
}

/// The verdict on one Spotify candidate. An `Option<f64>` collapsed "rejected
/// at 94.9" into the same value as "the duration was never plausible", so a
/// near miss worth tuning for read identically to a candidate that was junk.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Score {
    Match(f64),
    WrongVersion,
    WrongDuration(Duration),
    WrongTitle(f64),
    WrongArtist(f64),
}

impl Score {
    pub(crate) fn matched(&self) -> Option<f64> {
        match self {
            Self::Match(score) => Some(*score),
            Self::WrongVersion
            | Self::WrongDuration(_)
            | Self::WrongTitle(_)
            | Self::WrongArtist(_) => None,
        }
    }

    /// How close a rejected candidate came, so the nearest miss can be reported.
    /// A wrong version or duration is not a near miss at any title score.
    pub(crate) fn proximity(&self) -> f64 {
        match self {
            Self::Match(score) | Self::WrongTitle(score) | Self::WrongArtist(score) => *score,
            Self::WrongVersion | Self::WrongDuration(_) => 0.0,
        }
    }
}

impl std::fmt::Display for Score {
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
pub(crate) struct TrackMatcher<'a> {
    _track: &'a crate::types::Track,
    title: Title,
    core: StringMatcher<TrackTitle>,
    full: StringMatcher<TrackTitle>,
    artists: Artists,
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
        let title = Title::parse(&track.title);

        Ok(Self {
            _track: track,
            core: StringMatcher::new(title.core()),
            full: StringMatcher::new(&track.title),
            title,
            artists: Artists::parse(&track.artist.name),
            album: StringMatcher::new(&track.album.title),
            number: TrackNumMatcher::new(track.album_track_number),
            duration: TrackDurationMatcher::new(track.duration),
        })
    }

    /// Compare the parsed titles, but never below what the raw strings score:
    /// parsing may only add matches, never take one away.
    fn title_score(&mut self, result: &MatchParams) -> f64 {
        let core = self.core.score(result.title.core());
        let full = self.full.score(&result.title.original);

        // both comparisons carry the same type label, so name them here or the
        // two trace lines above are indistinguishable
        tracing::trace!(core, full, "title compared parsed and as written");

        core.max(full)
    }

    fn album_score(&mut self, result: &MatchParams) -> f64 {
        self.album.score(&result.album)
    }

    /// Performers credited in either title count toward the artist match:
    /// bandcamp writes them into the title where Spotify lists them as artists.
    /// Returns the best score and the candidate form that earned it.
    fn artist_score(&mut self, result: &MatchParams) -> (f64, String) {
        let subjects = self.artists.forms(&self.title.featured);
        let candidates = result.artists.forms(&result.title.featured);

        let mut best = (0f64, String::new());

        for subject in &subjects {
            let mut matcher = StringMatcher::<Artist>::new(subject);

            for candidate in &candidates {
                let score = matcher.score(candidate);
                if score > best.0 {
                    best = (score, candidate.clone());
                }
            }
        }

        tracing::trace!(
            subjects = ?subjects,
            candidates = ?candidates,
            best = %best.1,
            score = best.0,
            "compared artist forms",
        );

        best
    }

    fn track_number_score(&self, result: &MatchParams) -> f64 {
        self.number.score(result.number)
    }

    fn duration_score(&self, result: &MatchParams) -> Option<f64> {
        self.duration.score(result.duration)
    }

    pub(crate) fn score(&mut self, result: &SpotifyTrack) -> Score {
        let span = tracing::debug_span!(
            "candidate",
            id = result
                .id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_default(),
        );
        let _guard = span.enter();

        self.score_params(MatchParams::from(result))
    }

    fn score_params(&mut self, result: MatchParams) -> Score {
        let title = self.title_score(&result);
        let (artist, artist_form) = self.artist_score(&result);
        let album = self.album_score(&result);
        let tracknum = self.track_number_score(&result);
        let duration = self.duration_score(&result);

        let weighted = (title * TITLE_WEIGHT)
            + (artist * ARTIST_WEIGHT)
            + (album * ALBUM_WEIGHT)
            + (tracknum * TRACKNUM_WEIGHT)
            + (duration.unwrap_or_default() * DURATION_WEIGHT);

        let composite = (weighted / self.max_possible() as f64) * 100.0;

        // gates in the order they disqualify: a candidate rejected on version or
        // duration never earned a meaningful title score
        let verdict = if self.title.tags != result.title.tags {
            Score::WrongVersion
        } else if duration.is_none() {
            Score::WrongDuration(self.duration.duration.abs_diff(result.duration))
        } else if title < MIN_TITLE_SCORE {
            Score::WrongTitle(title)
        } else if artist < MIN_ARTIST_SCORE {
            Score::WrongArtist(artist)
        } else {
            Score::Match(composite)
        };

        // to a tenth: the remaining digits are float noise, not signal
        fn round(score: f64) -> f64 {
            (score * 10.0).round() / 10.0
        }

        tracing::debug!(
            cand.title = %result.title.original,
            cand.core = %result.title.core(),
            cand.artists = %result.artists.packed,
            cand.album = %result.album,
            cand.number = result.number,
            cand.secs = result.duration.as_secs_f64(),
            cand.tags = ?result.title.tags,
            score.title = round(title),
            score.artist = round(artist),
            score.artist_form = %artist_form,
            score.album = round(album),
            score.number = round(tracknum),
            score.duration = round(duration.unwrap_or_default()),
            score.composite = round(composite),
            limit.title = MIN_TITLE_SCORE,
            limit.artist = MIN_ARTIST_SCORE,
            limit.duration = ?self.duration.limit(),
            "{verdict}",
        );

        verdict
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
    fn normalize_folds_diacritics() {
        assert_eq!("desiree", normalize("Desirée"));
        assert_eq!("brujula", normalize("Brújula"));
        assert_eq!("ceska reka", normalize("Česká Řeka"));
        // stripping `Mn` at large would take the dakuten with it, turning が into か
        assert_eq!("君が目", normalize("君が目"));
    }

    #[test]
    fn normalize_short_titles() {
        // these bypassed normalization entirely under the 8-byte cutoff
        assert_eq!("wake", normalize("Wake."));
        assert_eq!("head", normalize("head!"));
        assert_eq!("001", normalize("001."));
    }

    #[test]
    fn token_set_scores() {
        // whole-token insertion is free once enough words are shared
        assert_eq!(
            1.0,
            token_set("small white animal", "small white animal 2026 remaster")
        );

        // but one shared word must not make a one-word title match anything
        assert!(token_set("title", "nope nope bad title") < 0.5);
        assert!(token_set("arise", "arise to the sun") < 0.5);
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
            // an unclosed group is not structure, so its text stays in the core
            ("Wake (up", "Wake up", vec![]),
            // a title that is nothing but a tag keeps its text
            ("(Instrumental)", "(Instrumental)", vec![]),
        ];

        for (raw, core, featured) in cases {
            let title = Title::parse(raw);
            assert_eq!(core, title.core(), "core of '{raw}'");
            assert_eq!(featured, title.featured, "featured of '{raw}'");
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
            // a different master is the same performance
            ("Small White Animal - 2026 Remaster", vec![]),
            ("Quartet (2022)", vec![]),
            ("Alone + Easy Target", vec![]),
        ];

        for (raw, tags) in cases {
            assert_eq!(tags, Title::parse(raw).tags, "tags of '{raw}'");
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
            let artists = Artists::parse(raw);
            assert_eq!(split, artists.split, "split of '{raw}'");
            assert_eq!(split[0], artists.lead(), "lead of '{raw}'");
        }

        assert!(Artists::parse("Various Artists").is_various());
        assert!(Artists::parse("V/A").is_various());
        assert!(!Artists::parse("Amulets").is_various());
    }

    #[test]
    fn title_credited_artist() {
        assert_eq!(
            Some(("Circadian Rhythm".to_string(), "Shela".to_string())),
            Title::parse("Circadian Rhythm - Shela").credited_artist()
        );
        assert_eq!(None, Title::parse("Shela").credited_artist());
    }

    #[test]
    fn track_matcher_packed_artist() {
        // bandcamp packs collaborators into one field; Spotify splits them, and
        // credits the featured performer bandcamp left in the title
        let track = {
            let mut track = types::Track::new(
                "Knew It All (Ft: Oddisee)",
                "Von Pea & The Other Guys",
                "album",
            );
            track.duration = types::Duration::from_secs(156);
            track.album_track_number = 7;
            track
        };

        let mut params = MatchParams::from(&track);
        params.title = Title::parse("Knew It All (feat. Oddisee)");
        params.artists = Artists::from_list(["Von Pea", "The Other Guys", "Oddisee"].into_iter());

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        assert_eq!(Score::Match(100.0), matcher.score_params(params));
    }

    #[test]
    fn track_matcher_rejects_other_version() {
        // a live take and the studio recording share a title and often a runtime
        let track = {
            let mut track = types::Track::new("Alone (Live in Berlin)", "artist", "album");
            track.duration = types::Duration::from_secs(240);
            track.album_track_number = 4;
            track
        };

        let mut params = MatchParams::from(&track);
        params.title = Title::parse("Alone");

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        assert_eq!(Score::WrongVersion, matcher.score_params(params));
    }

    #[test]
    fn track_matcher_exact() {
        let track = {
            let mut track = types::Track::new("track", "artist", "album");
            track.duration = types::Duration::from_secs(30);
            track.album_track_number = 2;
            track
        };

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        let score = matcher.score_params((&track).into());

        assert_eq!(Score::Match(100.0), score);
    }

    #[test]
    fn track_matcher_similar_title() {
        let track = {
            let mut track = types::Track::new("my track name!!", "artist", "album");
            track.duration = types::Duration::from_secs(30);
            track.album_track_number = 2;
            track
        };

        let other = {
            let mut other = track.clone();
            other.title = format!("{}!", other.title);
            other
        };

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        let score = matcher.score_params((&other).into());

        // both titles normalize to "my track name": every trailing `!` is
        // stripped, not just the last one
        assert_eq!(Score::Match(100.0), score);
    }

    #[test]
    fn track_matcher_wrong_title() {
        let track = {
            let mut track = types::Track::new("title", "artist", "album");
            track.duration = types::Duration::from_secs(30);
            track.album_track_number = 2;
            track
        };

        let other = {
            let mut other = track.clone();
            other.title = "nope nope bad title".to_string();
            other
        };

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        let score = matcher.score_params((&other).into());

        assert!(
            matches!(score, Score::WrongTitle(_)),
            "expected a title rejection, got {score:?}"
        );
    }

    #[test]
    fn duration_matcher() {
        let matcher = TrackDurationMatcher::new(types::Duration::from_secs(180));

        for secs in [178, 179, 180, 181, 182] {
            assert_eq!(
                Some(100.0),
                matcher.score(types::Duration::from_secs(secs)),
                "{secs}s should be indistinguishable from 180s"
            );
        }

        for secs in [0, 90, 195, 240, 360] {
            assert_eq!(
                None,
                matcher.score(types::Duration::from_secs(secs)),
                "{secs}s should be disqualified"
            );
        }

        let near = matcher
            .score(types::Duration::from_secs(185))
            .expect("185s is inside the limit");
        let far = matcher
            .score(types::Duration::from_secs(192))
            .expect("192s is inside the limit");

        assert!(near > far, "185s ({near}) should outrank 192s ({far})");
    }

    #[test]
    fn duration_matcher_scales_with_length() {
        // 15s is a sixth of a 90s track, so short tracks keep the flat floor
        let short = TrackDurationMatcher::new(types::Duration::from_secs(90));
        assert_eq!(types::Duration::from_secs(15), short.limit());

        // ...but it is 1% of a twenty-minute piece, where masters drift further
        let long = TrackDurationMatcher::new(types::Duration::from_secs(1200));
        assert_eq!(types::Duration::from_secs(60), long.limit());

        assert!(
            long.score(types::Duration::from_secs(1250)).is_some(),
            "50s off a 20 minute track is within 5%"
        );
        assert_eq!(
            None,
            long.score(types::Duration::from_secs(1270)),
            "70s off a 20 minute track is not"
        );
    }

    #[test]
    fn track_matcher_wrong_duration() {
        let track = {
            let mut track = types::Track::new("title", "artist", "album");
            track.duration = types::Duration::from_secs(180);
            track.album_track_number = 2;
            track
        };

        let other = {
            let mut other = track.clone();
            other.duration = types::Duration::from_secs(240);
            other
        };

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        let score = matcher.score_params((&other).into());

        assert!(
            matches!(score, Score::WrongDuration(_)),
            "expected a duration rejection, got {score:?}"
        );
    }

    #[test]
    fn track_matcher_wrong_artist() {
        let track = {
            let mut track = types::Track::new("title", "artist", "album");
            track.duration = types::Duration::from_secs(30);
            track.album_track_number = 2;
            track
        };

        let other = {
            let mut other = track.clone();
            other.artist = types::Artist::new("nope not the right artist");
            other
        };

        let mut matcher = TrackMatcher::new(&track).expect("should not fail");

        let score = matcher.score_params((&other).into());

        assert!(
            matches!(score, Score::WrongArtist(_)),
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
            let track = {
                let mut track = types::Track::new("title", case.0, "album");
                track.duration = types::Duration::from_secs(30);
                track.album_track_number = 2;
                track
            };

            let mut params = MatchParams::from(&track);
            params.artists = Artists::from_list(case.1.iter().copied());

            let mut matcher = TrackMatcher::new(&track).expect("should not fail");

            let score = matcher.score_params(params);

            assert_eq!(
                Score::Match(100.0),
                score,
                "track: '{}', result: '{:?}'",
                case.0,
                case.1
            );
        }
    }
}
