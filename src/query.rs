use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Query(String);

impl Query {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Query {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrackQuery {
    Any(Query),
    InPost { post: Query, number: u32 },
}

impl TrackQuery {
    fn in_post((post, number): (&str, &str)) -> Option<Self> {
        if post.is_empty() {
            return None;
        }

        Some(Self::InPost {
            post: Query(post.to_string()),
            number: number.parse().ok()?,
        })
    }
}

impl FromStr for TrackQuery {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("http") {
            return Ok(Self::Any(Query(s.to_string())));
        }

        if let Some(query) = s.rsplit_once('/').and_then(Self::in_post) {
            return Ok(query);
        }

        if let Some(query) = s
            .rsplit_once('.')
            .filter(|(post, _)| post.contains('.'))
            .and_then(Self::in_post)
        {
            return Ok(query);
        }

        Ok(Self::Any(Query(s.to_string())))
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
