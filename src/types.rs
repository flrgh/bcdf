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
    pub(crate) album: Album,
    pub(crate) duration: Duration,
    pub(crate) number: usize,
    pub(crate) bandcamp_playlist_track_number: usize,
    pub(crate) download_url: Option<String>,
    pub(crate) bandcamp_track_id: Option<String>,
    pub(crate) spotify_id: Option<String>,
}

impl Track {
    pub(crate) fn filename(&self) -> PathBuf {
        let title = self.title.replace('/', "_");
        let fname = format!("{:02} - {} - {}.mp3", self.bandcamp_playlist_track_number, self.artist.name, title);
        println!("{fname}");
        PathBuf::from(fname)
    }
}
