mod bandcamp;
mod cli;
mod compress;
mod db;
mod download;
mod http;
mod list;
mod log;
mod metrics;
mod mp3;
mod playlist;
mod post;
mod query;
mod rename;
mod scan;
mod scrape;
mod search;
mod spotify;
mod tag;
mod track;

use anyhow::Context;
use std::path::PathBuf;

use crate::db::*;

pub(crate) mod types {
    pub(crate) use std::time::Duration;
    pub(crate) type DateTime = chrono::DateTime<chrono::Utc>;
    pub(crate) type SpotifyTrack = rspotify::model::FullTrack;
}

pub(crate) const DEFAULT_DATA_DIR: &str = "./data";

#[derive(Debug)]
pub(crate) struct App {
    pub(crate) root: PathBuf,
    pub(crate) db: Db,
}

impl App {
    pub(crate) async fn open<T: AsRef<std::path::Path>>(dir: T) -> anyhow::Result<Self> {
        let root = PathBuf::from(dir.as_ref());

        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating state directory {root:?}"))?;

        let db = open(root.join("state.db")).await?;

        Ok(Self { root, db })
    }

    pub(crate) fn path<T: AsRef<std::path::Path>>(&self, relative: T) -> PathBuf {
        self.root.join(relative)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::new();
    let app = App::open(&cli.data_dir).await?;
    cli.exec(app).await
}
