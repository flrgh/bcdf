use std::path::PathBuf;
pub(crate) use std::time::Duration;
pub(crate) type DateTime = chrono::DateTime<chrono::Utc>;

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Track {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: String,
    pub(crate) duration: Duration,
    pub(crate) number: usize,
    pub(crate) download_url: Option<String>,
}

impl Track {
    pub(crate) fn filename(&self) -> PathBuf {
        PathBuf::from(format!("{} - {} - {}", self.number, self.artist, self.title))
    }
}
