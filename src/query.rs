use crate::track::TrackRow;
use crate::types::BlogPost;
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;
use unicode_normalization::UnicodeNormalization;

pub(crate) const DATE_FORMAT: &str = "%Y-%m-%d";

pub(crate) fn normalize(s: &str) -> String {
    s.nfc()
        .map(|c| match c {
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{2018}' | '\u{2019}' | '\u{201b}' | '\u{2032}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{201f}' | '\u{2033}' => '"',
            c => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) trait Matchable {
    /// normalized string values, compared with substring match
    fn search_fields(&self) -> Vec<Cow<'_, str>>;

    /// [mostly] unique fields, compared with ==
    fn identity_fields(&self) -> Vec<Cow<'_, str>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Query {
    raw: String,
    normalized: String,
}

impl Query {
    pub(crate) fn new(raw: &str) -> Self {
        Self {
            raw: raw.to_string(),
            normalized: normalize(raw),
        }
    }

    fn matches_exact<T: Matchable>(&self, item: &T) -> bool {
        item.identity_fields()
            .iter()
            .any(|field| field.as_ref() == self.raw)
    }

    fn matches_partial<T: Matchable>(&self, item: &T) -> bool {
        item.search_fields()
            .iter()
            .any(|field| normalize(field).contains(&self.normalized))
    }

    pub(crate) fn matches<T: Matchable>(&self, item: &T) -> bool {
        self.matches_exact(item) || self.matches_partial(item)
    }

    pub(crate) fn filter<'a, T: Matchable>(&self, items: &'a [T]) -> Vec<&'a T> {
        let exact: Vec<&T> = items
            .iter()
            .filter(|item| self.matches_exact(*item))
            .collect();

        if !exact.is_empty() {
            return exact;
        }

        items
            .iter()
            .filter(|item| self.matches_partial(*item))
            .collect()
    }

    pub(crate) fn filter_unique<'a, T: Matchable>(&self, items: &'a [T]) -> anyhow::Result<&'a T> {
        unique(self.filter(items), self)
    }
}

fn unique<T>(mut matches: Vec<T>, query: &dyn fmt::Display) -> anyhow::Result<T> {
    match matches.len() {
        0 => anyhow::bail!("no item matched {query}"),
        1 => Ok(matches.remove(0)),
        n => anyhow::bail!("{n} items matched {query}"),
    }
}

impl FromStr for Query {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrackQuery {
    Any(Query),
    InPost { post: Query, number: usize },
}

impl TrackQuery {
    pub(crate) fn filter<'a>(&self, posts: &'a [BlogPost]) -> Vec<TrackRow<'a>> {
        match self {
            Self::Any(query) => TrackRow::all(posts)
                .into_iter()
                .filter(|row| query.matches(row))
                .collect(),

            Self::InPost { post, number } => Self::in_post(post, *number, posts),
        }
    }

    pub(crate) fn filter_unique<'a>(&self, posts: &'a [BlogPost]) -> anyhow::Result<TrackRow<'a>> {
        let matches: Vec<TrackRow<'a>> = match self {
            Self::Any(query) => {
                let rows = TrackRow::all(posts);
                query.filter(&rows).into_iter().copied().collect()
            }
            Self::InPost { post, number } => Self::in_post(post, *number, posts),
        };

        unique(matches, self)
    }

    fn in_post<'a>(post: &Query, number: usize, posts: &'a [BlogPost]) -> Vec<TrackRow<'a>> {
        post.filter(posts)
            .into_iter()
            .filter_map(|post| {
                post.tracks
                    .iter()
                    .find(|track| track.post_track_number == number)
                    .map(|track| TrackRow::new(post, track))
            })
            .collect()
    }
}

impl FromStr for TrackQuery {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let split = if s.starts_with("http") {
            None
        } else {
            s.rsplit_once('/')
        };

        Ok(match split {
            Some((post, number)) if !post.is_empty() => match number.parse() {
                Ok(number) => Self::InPost {
                    post: Query::new(post),
                    number,
                },
                Err(_) => Self::Any(Query::new(s)),
            },
            _ => Self::Any(Query::new(s)),
        })
    }
}

impl fmt::Display for TrackQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any(query) => query.fmt(f),
            Self::InPost { post, number } => write!(f, "{post}/{number}"),
        }
    }
}
