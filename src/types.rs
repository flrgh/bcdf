pub(crate) use std::time::Duration;
pub(crate) type DateTime = chrono::DateTime<chrono::Utc>;
pub(crate) type SpotifyTrack = rspotify::model::FullTrack;

/// A bandcamp daily post, its tracks, and the Spotify playlist they belong to.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct BlogPost {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) published: DateTime,
    pub(crate) modified: DateTime,

    pub(crate) dir: String,

    pub(crate) tracks: Vec<Track>,
    pub(crate) spotify_playlist: Option<SpotifyPlaylist>,
}

impl BlogPost {
    pub(crate) fn derive_post_dir(published: &DateTime, url: &str) -> String {
        let path = url
            .strip_suffix('/')
            .unwrap_or(url)
            .rsplit('/')
            .next()
            .expect("bandamp post url has at least one path component");

        assert!(!path.is_empty(), "empty post slug for {url}");
        format!("{}-{}", published.format("%Y-%m-%d"), path).replace('/', "_")
    }

    pub(crate) fn derive_dir(&self) -> String {
        Self::derive_post_dir(&self.published, &self.url)
    }

    pub(crate) fn has_spotify_tracks(&self) -> bool {
        self.tracks.iter().any(|t| t.spotify_id.is_some())
    }

    pub(crate) fn needs_playlist_assignments(&self) -> bool {
        self.tracks
            .iter()
            .any(|t| t.spotify_id.is_some() && t.spotify_playlist_id.is_none())
    }

    pub(crate) fn downloaded_count(&self) -> usize {
        self.tracks.iter().filter(|t| t.filename.is_some()).count()
    }

    pub(crate) fn spotify_count(&self) -> usize {
        self.tracks
            .iter()
            .filter(|t| t.spotify_id.is_some())
            .count()
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct BlogPostRow {
    pub(crate) post: BlogPost,

    /// YYYY-MM-DD.NN, where YYYY-MM-DD is the post's publish date, and NN is an
    /// ordinal number--the nth post published on that date.
    pub(crate) locator: String,
}

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize)]
pub(crate) struct SpotifyPlaylist {
    pub(crate) id: String,

    /// The name the playlist was created with. Spotify offers no lookup by id
    /// for a playlist we may not own yet, so we re-find it by name -- and a
    /// name re-derived from a since-retitled post finds nothing and creates a
    /// duplicate.
    pub(crate) name: String,
}

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize)]
pub(crate) struct Artist {
    pub(crate) name: String,
    pub(crate) bandcamp_id: Option<u64>,
    pub(crate) bandcamp_url: Option<String>,
    pub(crate) spotify_id: Option<String>,
}

#[cfg(test)]
impl Artist {
    pub(crate) fn new<T: AsRef<str>>(name: T) -> Self {
        Self {
            name: name.as_ref().to_string(),
            bandcamp_id: Default::default(),
            bandcamp_url: Default::default(),
            spotify_id: Default::default(),
        }
    }
}

#[cfg(test)]
impl<T> From<T> for Artist
where
    T: AsRef<str>,
{
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize)]
pub(crate) struct Album {
    pub(crate) title: String,
    pub(crate) bandcamp_id: Option<u64>,
    pub(crate) bandcamp_url: Option<String>,
    pub(crate) spotify_id: Option<String>,
}

#[cfg(test)]
impl Album {
    pub(crate) fn new<T: AsRef<str>>(title: T) -> Self {
        Self {
            title: title.as_ref().to_string(),
            bandcamp_id: Default::default(),
            bandcamp_url: Default::default(),
            spotify_id: Default::default(),
        }
    }
}

#[cfg(test)]
impl<T> From<T> for Album
where
    T: AsRef<str>,
{
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Track {
    pub(crate) title: String,
    pub(crate) artist: Artist,
    pub(crate) album_artist: Artist,
    pub(crate) album: Album,
    pub(crate) duration: Duration,
    pub(crate) album_track_number: usize,
    pub(crate) post_track_number: usize,
    pub(crate) download_url: Option<String>,
    pub(crate) bandcamp_id: u64,
    pub(crate) spotify_id: Option<String>,
    pub(crate) spotify_match_score: Option<f64>,
    pub(crate) spotify_playlist_id: Option<String>,
    pub(crate) filename: Option<String>,
}

#[cfg(test)]
impl Track {
    pub(crate) fn new<T, AT, AL>(title: T, artist: AT, album: AL) -> Self
    where
        T: AsRef<str>,
        AT: Into<Artist> + Clone,
        AL: Into<Album>,
    {
        Self {
            title: title.as_ref().to_string(),
            artist: artist.clone().into(),
            album_artist: artist.into(),
            album: album.into(),
            duration: Default::default(),
            album_track_number: Default::default(),
            post_track_number: Default::default(),
            download_url: Default::default(),
            bandcamp_id: Default::default(),
            spotify_id: Default::default(),
            spotify_match_score: Default::default(),
            spotify_playlist_id: Default::default(),
            filename: Default::default(),
        }
    }
}

impl Track {
    pub(crate) fn derive_filename(&self) -> String {
        format!(
            "{:02} - {} - {}.mp3",
            self.post_track_number, self.artist.name, self.title
        )
        .replace('/', "_")
        .replace('\n', "_")
    }
}

/// Rendering mode for the browse commands
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Table,
    Json,
}
