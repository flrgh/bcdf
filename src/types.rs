use std::path::PathBuf;
pub(crate) use std::time::Duration;
pub(crate) type DateTime = chrono::DateTime<chrono::Utc>;

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Artist {
    pub(crate) name: String,
    pub(crate) bandcamp_id: Option<String>,
    pub(crate) bandcamp_url: Option<String>,
    pub(crate) spotify_id: Option<String>,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Album {
    pub(crate) title: String,
    pub(crate) bandcamp_id: Option<String>,
    pub(crate) bandcamp_url: Option<String>,
    pub(crate) spotify_id: Option<String>,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
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
}
