use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub(crate) struct Args {
    /// Base directory for storing state and downloaded mp3 files
    #[arg(long, value_name = "DIR", default_value = crate::store::DEFAULT_DATA_DIR)]
    pub(crate) data_dir: PathBuf,

    /// Don't download anything
    #[arg(long, default_value_t = false)]
    pub(crate) no_download: bool,

    /// Don't create Spotify playlists
    #[arg(long, default_value_t = false)]
    pub(crate) no_spotify: bool,

    /// Scan only a single url
    #[arg(long)]
    pub(crate) url: Option<String>,

    /// Re-scan from the filesystem only
    #[arg(long, default_value_t = false)]
    pub(crate) rescan: bool,
}

pub(crate) fn args() -> Args {
    Args::parse()
}
