use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Table,
    Json,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub(crate) struct Cli {
    /// Base directory for storing state and downloaded mp3 files
    #[arg(long, global = true, value_name = "DIR", default_value = crate::DEFAULT_DATA_DIR)]
    pub(crate) data_dir: PathBuf,

    #[command(subcommand)]
    command: Command,

    #[command(flatten)]
    logging: crate::log::Logging,
}

impl Cli {
    pub(crate) fn new() -> Self {
        let cli = Self::parse();
        cli.logging.init();
        cli
    }

    pub(crate) async fn exec(self, app: crate::App) -> anyhow::Result<()> {
        match self.command {
            Command::Scan(scan) => scan.exec(&app).await,
            Command::Mp3(mp3) => mp3.exec(&app).await,
            Command::Post(post) => post.exec(&app).await,
            Command::Track(track) => track.exec(&app).await,
            Command::Playlist(playlist) => playlist.exec(&app).await,
            Command::Spotify(spotify) => spotify.exec().await,
            Command::Scrape(cli) => cli.exec(&app).await,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    Scan(crate::scan::Cli),
    Mp3(crate::mp3::Cli),
    Post(crate::post::Cli),
    Track(crate::track::Cli),
    Playlist(crate::playlist::Cli),
    Spotify(crate::spotify::Cli),
    Scrape(crate::scrape::Cli),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn data_dir_parses_before_and_after_command() {
        for argv in [
            ["bcdf", "--data-dir", "/x", "mp3", "tag"],
            ["bcdf", "mp3", "tag", "--data-dir", "/x"],
        ] {
            let cli = Cli::try_parse_from(argv).unwrap();
            assert_eq!(cli.data_dir, PathBuf::from("/x"));
        }
    }
}
