use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub(crate) struct Args {
    /// Base directory for storing downloaded content
    #[arg(long, value_name = "PATH", default_value_t = crate::state::OUT_DIR.to_string())]
    pub(crate) download_to: String,

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
