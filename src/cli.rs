use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub(crate) struct Cli {
    /// Base directory for storing state and downloaded mp3 files
    #[arg(long, global = true, value_name = "DIR", default_value = crate::store::DEFAULT_DATA_DIR)]
    data_dir: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    run: crate::run::Cli,
}

impl Cli {
    fn resolved_command(&mut self) -> Command {
        self.command.take().unwrap_or(Command::Run)
    }

    pub(crate) async fn exec(mut self) -> anyhow::Result<()> {
        let mut store = crate::store::Store::open(&self.data_dir)?;

        match self.resolved_command() {
            Command::Run => self.run.exec(&mut store).await,
            Command::Mp3(mp3) => mp3.exec(&store).await,
        }
    }
}

pub(crate) async fn run() -> anyhow::Result<()> {
    Cli::parse().exec().await
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scrape new posts, download their mp3s, and update Spotify playlists
    Run,

    Mp3(crate::mp3::Cli),
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
    fn bare_invocation_resolves_to_run() {
        let mut cli = Cli::try_parse_from(["bcdf"]).unwrap();
        assert!(matches!(cli.resolved_command(), Command::Run));
    }

    #[test]
    fn named_run_resolves_to_run() {
        let mut cli = Cli::try_parse_from(["bcdf", "run"]).unwrap();
        assert!(matches!(cli.resolved_command(), Command::Run));
    }

    #[test]
    fn run_flags_bind_identically_on_either_side_of_the_subcommand() {
        let unset = Cli::try_parse_from(["bcdf"]).unwrap();
        let bare = Cli::try_parse_from(["bcdf", "--rescan"]).unwrap();
        let before = Cli::try_parse_from(["bcdf", "--rescan", "run"]).unwrap();
        let after = Cli::try_parse_from(["bcdf", "run", "--rescan"]).unwrap();

        assert_ne!(unset.run, bare.run);
        assert_eq!(bare.run, before.run);
        assert_eq!(bare.run, after.run);
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
