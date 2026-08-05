use std::path::PathBuf;
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

    /// Where the post's mp3s live, relative to the store root. Derived from
    /// the post when it is first seen and fixed thereafter, so a retitle
    /// upstream doesn't orphan the files.
    pub(crate) dir: PathBuf,

    pub(crate) tracks: Vec<Track>,
    pub(crate) spotify_playlist: Option<SpotifyPlaylist>,
}

impl BlogPost {
    pub(crate) fn has_spotify_tracks(&self) -> bool {
        self.tracks.iter().any(|t| t.spotify_id.is_some())
    }

    pub(crate) fn needs_playlist_assignments(&self) -> bool {
        self.tracks
            .iter()
            .any(|t| t.spotify_id.is_some() && t.spotify_playlist_id.is_none())
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub(crate) struct SpotifyPlaylist {
    pub(crate) id: String,

    /// The name the playlist was created with. Spotify offers no lookup by id
    /// for a playlist we may not own yet, so we re-find it by name -- and a
    /// name re-derived from a since-retitled post finds nothing and creates a
    /// duplicate.
    pub(crate) name: String,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub(crate) struct Artist {
    pub(crate) name: String,
    pub(crate) bandcamp_id: Option<String>,
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

#[derive(Debug, Eq, PartialEq, Clone)]
pub(crate) struct Album {
    pub(crate) title: String,
    pub(crate) bandcamp_id: Option<String>,
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
    pub(crate) bandcamp_id: Option<String>,
    pub(crate) spotify_id: Option<String>,
    pub(crate) spotify_match_score: Option<f64>,
    pub(crate) spotify_playlist_id: Option<String>,
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
        }
    }
}

impl Track {
    pub(crate) fn mp3_filename(&self) -> PathBuf {
        let title = self.title.replace('/', "_");
        let artist = self.artist.name.replace('/', "_");
        PathBuf::from(format!(
            "{:02} - {} - {}.mp3",
            self.post_track_number, artist, title
        ))
    }
}
