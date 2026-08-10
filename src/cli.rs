use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub(crate) struct Cli {
    /// Base directory for storing state and downloaded mp3 files
    #[arg(long, global = true, value_name = "DIR", default_value = crate::store::DEFAULT_DATA_DIR)]
    data_dir: PathBuf,

    #[command(subcommand)]
    command: Command,

    #[command(flatten)]
    logger: crate::log::Logger,
}

impl Cli {
    pub(crate) fn new() -> Self {
        Self::parse()
    }

    pub(crate) async fn exec(self) -> anyhow::Result<()> {
        self.logger.init();

        let mut store = crate::store::Store::open(&self.data_dir)?;

        match self.command {
            Command::Scan(scan) => scan.exec(&mut store).await,
            Command::Mp3(mp3) => mp3.exec(&mut store).await,
            Command::Post(post) => post.exec(&store),
            Command::Track(track) => track.exec(&store).await,
            Command::Playlist(playlist) => playlist.exec(&store),
            Command::Spotify(spotify) => spotify.exec(&store).await,
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
