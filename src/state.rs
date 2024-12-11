use crate::{bandcamp::BlogInfo, types::Track};
use serde_json as json;
use std::path::{Path, PathBuf};

pub(crate) const OUT_DIR: &str = "./data";
const BLOG_INFO_FILENAME: &str = "info.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct State {
    pub(crate) blog_info: BlogInfo,
    pub(crate) tracks: Vec<Track>,
    pub(crate) spotify_playlist_id: Option<String>,
    root_dir: PathBuf,
}

fn dirname(info: &BlogInfo, dir: &Path) -> PathBuf {
    dir.join(format!(
        "{} - {}",
        info.published.format("%Y-%m-%d"),
        info.title
    ))
}

fn filename(info: &BlogInfo, dir: &Path) -> PathBuf {
    dirname(info, dir).join(BLOG_INFO_FILENAME)
}

pub(crate) fn save<T: serde::Serialize>(t: &T, fname: &PathBuf) -> anyhow::Result<()> {
    if let Some(dir) = fname.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let fh = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(fname)?;

    Ok(serde_json::to_writer(fh, t)?)
}

pub(crate) fn load<T: for<'de> serde::Deserialize<'de>>(fname: &PathBuf) -> anyhow::Result<T> {
    if let Some(dir) = fname.parent() {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }
    }

    let fh = std::fs::OpenOptions::new()
        .read(true)
        .create(false)
        .create_new(false)
        .open(fname)?;

    Ok(serde_json::from_reader::<_, T>(fh)?)
}

impl State {
    fn new(info: BlogInfo) -> Self {
        Self {
            blog_info: info,
            tracks: vec![],
            spotify_playlist_id: None,
            root_dir: OUT_DIR.into(),
        }
    }

    pub(crate) fn filename(&self) -> PathBuf {
        filename(&self.blog_info, &self.root_dir)
    }

    pub(crate) fn dirname(&self) -> PathBuf {
        dirname(&self.blog_info, &self.root_dir)
    }

    pub(crate) fn rehydrate_tracks(&mut self) -> anyhow::Result<()> {
        let mut tracks = Vec::with_capacity(self.blog_info.tracks.len());

        let dir = self.dirname();

        for new in self.blog_info.tracks.iter() {
            let fname = dir.join(new.meta_filename());

            let mut track = load::<Track>(&fname).unwrap_or_else(|_err| new.clone());
            track.download_url = new.download_url.clone().or(track.download_url);

            save(&track, &fname)?;
            tracks.push(track);
        }

        self.tracks = tracks;
        Ok(())
    }

    pub(crate) fn try_get_or_create(info: BlogInfo, dir: &str) -> anyhow::Result<Self> {
        let dir = PathBuf::from(dir);
        let path = filename(&info, &dir);

        let mut state = if let Ok(mut state) = load::<Self>(&path) {
            state.blog_info = info;
            state
        } else {
            Self::new(info)
        };

        state.root_dir = dir;

        state.rehydrate_tracks()?;
        Ok(state)
    }

    pub(crate) fn save(&self) -> anyhow::Result<()> {
        let mut fh = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(self.filename())?;

        json::to_writer(&mut fh, &self)?;

        let dir = self.dirname();
        for track in &self.tracks {
            save(track, &dir.join(track.meta_filename()))?;
        }

        Ok(())
    }

    pub(crate) fn has_spotify_tracks(&self) -> bool {
        self.tracks.iter().any(|t| t.spotify_id.is_some())
    }

    pub(crate) fn needs_playlist_assignments(&self) -> bool {
        self.tracks
            .iter()
            .any(|t| t.spotify_id.is_some() && t.spotify_playlist_id.is_none())
    }

    pub(crate) fn needs_spotify_updates(&self) -> bool {
        self.spotify_playlist_id.is_none()
            || self
                .tracks
                .iter()
                .any(|t| t.spotify_id.is_none() || t.spotify_playlist_id.is_none())
    }

    pub(crate) fn needs_downloads(&self) -> bool {
        self.tracks.iter().any(|track| {
            let path = self.dirname().join(track.mp3_filename());
            !path.exists()
        })
    }
}

pub(crate) fn blog_urls(args: &crate::cli::Args) -> anyhow::Result<Vec<String>> {
    let res = std::fs::read_dir(&args.download_to)?;

    let mut urls = Vec::new();

    for child in res {
        let child = child?;
        let fname = child.path().join(BLOG_INFO_FILENAME);

        let Ok(state) = load::<State>(&fname) else {
            continue;
        };

        if !args.no_spotify && state.needs_spotify_updates() {
            urls.push(state.blog_info.url);
            continue;
        }

        if !args.no_download && state.needs_downloads() {
            urls.push(state.blog_info.url);
            continue;
        }
    }

    Ok(urls)
}
