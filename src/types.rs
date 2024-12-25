use std::path::{Path, PathBuf};
pub(crate) use std::time::Duration;
pub(crate) type DateTime = chrono::DateTime<chrono::Utc>;
pub(crate) type SpotifyTrack = rspotify::model::FullTrack;

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Track {
    pub(crate) title: String,
    pub(crate) artist: Artist,
    pub(crate) album_artist: Artist,
    pub(crate) album: Album,
    pub(crate) duration: Duration,
    pub(crate) number: usize,
    pub(crate) bandcamp_playlist_track_number: usize,
    pub(crate) download_url: Option<String>,
    pub(crate) bandcamp_track_id: Option<String>,
    pub(crate) spotify_id: Option<String>,
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
            number: Default::default(),
            bandcamp_playlist_track_number: Default::default(),
            download_url: Default::default(),
            bandcamp_track_id: Default::default(),
            spotify_id: Default::default(),
            spotify_playlist_id: Default::default(),
        }
    }
}

pub(crate) fn update<T: Clone + Eq>(old: &mut Option<T>, other: &Option<T>) -> bool {
    if other.is_some() && old != other {
        *old = other.clone();
        true
    } else {
        false
    }
}

impl Track {
    pub(crate) fn filename(&self, ext: &str) -> PathBuf {
        let title = self.title.replace('/', "_");
        let artist = self.artist.name.replace('/', "_");
        let fname = format!(
            "{:02} - {} - {}.{}",
            self.bandcamp_playlist_track_number, artist, title, ext
        );
        PathBuf::from(fname)
    }

    pub(crate) fn mp3_filename(&self) -> PathBuf {
        self.filename("mp3")
    }

    pub(crate) fn meta_filename(&self) -> PathBuf {
        self.filename("json")
    }

    pub(crate) fn rehydrate(&mut self, from_disk: Track, fname: &Path) {
        let mp3 = fname.with_extension("mp3");
        if mp3.exists() {
            // we already downloaded the mp3 successfully, so restore
            // the existing download url
            self.download_url = from_disk.download_url;
        }

        self.spotify_id = from_disk.spotify_id;
        self.spotify_playlist_id = from_disk.spotify_playlist_id;
    }
}
