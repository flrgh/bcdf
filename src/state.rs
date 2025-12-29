use crate::{bandcamp::BlogPost, types::Track};
use serde_json as json;
use std::path::{Path, PathBuf};

pub(crate) const OUT_DIR: &str = "./data";
const BLOG_INFO_FILENAME: &str = "info.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct State {
    pub(crate) blog_info: BlogPost,
    pub(crate) tracks: Vec<Track>,
    pub(crate) spotify_playlist_id: Option<String>,
    root_dir: PathBuf,

    #[serde(skip_serializing, default)]
    need_save: bool,

    #[serde(skip_serializing, default)]
    need_save_tracks: bool,
}

fn dirname(info: &BlogPost, dir: &Path) -> PathBuf {
    dir.join(format!(
        "{} - {}",
        info.published.format("%Y-%m-%d"),
        info.title
    ))
}

fn filename(info: &BlogPost, dir: &Path) -> PathBuf {
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

pub(crate) fn update<T>(t: &T, fname: &PathBuf) -> anyhow::Result<()>
where
    T: serde::Serialize,
    T: for<'de> serde::Deserialize<'de>,
    T: Eq,
{
    if let Ok(current) = load::<T>(fname) {
        if &current == t {
            return Ok(());
        }
    }

    save(t, fname)
}

fn remove_extraneous(dir: &Path, track: &Track) -> anyhow::Result<()> {
    let dupe_meta = dir.join(track.meta_filename());
    if dupe_meta.exists() {
        tracing::warn!("removing extraneous track metadata file: {:?}", dupe_meta);
        std::fs::remove_file(dupe_meta)?;
    }

    let dupe_mp3 = dir.join(track.mp3_filename());
    if dupe_mp3.exists() {
        tracing::warn!("removing extraneous track mp3 file: {:?}", dupe_mp3,);
        std::fs::remove_file(dupe_mp3)?;
    }

    Ok(())
}

fn consolidate(dir: &Path, keep: &Track, dupe: &Track) -> anyhow::Result<()> {
    let keep_meta = dir.join(keep.meta_filename());
    let keep_mp3 = dir.join(keep.mp3_filename());

    let dupe_meta = dir.join(dupe.meta_filename());
    let dupe_mp3 = dir.join(dupe.mp3_filename());

    if keep_meta == dupe_meta || keep_mp3 == dupe_mp3 {
        return Ok(());
    }

    if keep_meta.exists() {
        tracing::warn!("removing duplicate track metadata file: {:?}", dupe_meta,);
        std::fs::remove_file(dupe_meta)?;

        if keep_mp3.exists() && dupe_mp3.exists() {
            tracing::warn!("removing duplicate track mp3 file: {:?}", dupe_mp3,);
            std::fs::remove_file(dupe_mp3)?;
        }
    }

    Ok(())
}

impl State {
    fn new(info: BlogPost) -> Self {
        Self {
            blog_info: info,
            tracks: vec![],
            spotify_playlist_id: None,
            root_dir: OUT_DIR.into(),
            need_save: true,
            need_save_tracks: true,
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

            let mut new = new.clone();

            match load::<Track>(&fname) {
                Ok(track) => {
                    new.rehydrate(track, &fname);
                }
                Err(e) => {
                    tracing::debug!("failed to load track from {fname:?}: {e}");
                }
            };

            tracks.push(new);
        }

        self.tracks = tracks;
        Ok(())
    }

    pub(crate) fn try_get_or_create(info: BlogPost, dir: &str) -> anyhow::Result<Self> {
        let dir = PathBuf::from(dir);
        let path = filename(&info, &dir);

        let mut state = if let Ok(mut state) = load::<Self>(&path) {
            state.blog_info = info;
            state.need_save = true;
            state
        } else {
            Self::new(info)
        };

        state.root_dir = dir;

        state.rehydrate_tracks()?;
        state.cleanup_files()?;
        Ok(state)
    }

    pub(crate) fn save(&mut self) -> anyhow::Result<()> {
        if self.need_save {
            let mut fh = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .create(true)
                .open(self.filename())?;

            json::to_writer(&mut fh, &self)?;

            self.need_save = false;
        }

        if self.need_save_tracks {
            let dir = self.dirname();
            for track in &self.tracks {
                update(track, &dir.join(track.meta_filename()))?;
            }

            self.need_save_tracks = false;
        }

        Ok(())
    }

    pub(crate) fn need_save(&mut self) {
        self.need_save = true;
    }

    pub(crate) fn need_save_tracks(&mut self) {
        self.need_save_tracks = true;
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

    fn cleanup_files(&self) -> anyhow::Result<()> {
        let dir = self.dirname();

        let paths = std::fs::read_dir(&dir)?.filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.exists()
                && path.is_file()
                && path.extension().is_some_and(|ext| ext == "json")
                && !path.ends_with(BLOG_INFO_FILENAME)
            {
                Some(path)
            } else {
                None
            }
        });

        for path in paths {
            let track = match load::<Track>(&path) {
                Ok(track) => track,
                Err(e) => {
                    tracing::warn!("failed loading metadata file {path:?}: {e}");
                    continue;
                }
            };

            if self
                .tracks
                .iter()
                .any(|other| other.meta_filename() == track.meta_filename())
            {
                continue;
            }

            if let Some(other) = self.tracks.iter().find(|other| {
                other.title == track.title
                    || (other.bandcamp_track_id.is_some()
                        && other.bandcamp_track_id == track.bandcamp_track_id)
            }) {
                consolidate(&dir, other, &track)?;
            } else {
                remove_extraneous(&dir, &track)?;
            }
        }

        Ok(())
    }
}

pub(crate) fn load_blogs(dir: &str) -> anyhow::Result<Vec<State>> {
    Ok(std::fs::read_dir(dir)?
        .filter_map(|child| {
            let child = child.ok()?;
            let fname = child.path().join(BLOG_INFO_FILENAME);
            let mut state = load::<State>(&fname).ok()?;
            state.rehydrate_tracks().ok()?;
            if let Err(e) = state.cleanup_files() {
                tracing::warn!(
                    "Error with filename normalization for {}: {}",
                    state.blog_info.title,
                    e,
                );
            }
            Some(state)
        })
        .collect())
}

pub(crate) fn blog_urls(args: &crate::cli::Args) -> anyhow::Result<Vec<String>> {
    let states = load_blogs(&args.download_to)?;

    let mut urls = Vec::with_capacity(states.len());

    for state in states.into_iter() {
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
