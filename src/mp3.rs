use crate::store::Store;
use crate::{download, rename, tag};
use clap::Subcommand;

/// Manage downloaded mp3 files
#[derive(clap::Args, Debug)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub(crate) async fn exec(self, store: &mut Store) -> anyhow::Result<()> {
        match self.command {
            Command::Download => {
                for mut row in store.list_posts()? {
                    download::download(store, &mut row.post).await?;
                }
            }
            Command::Tag => {
                for row in store.list_posts()? {
                    tag::tag(store, &row.post).await?;
                }
            }
            Command::Rename { dry_run } => rename::rename(store, dry_run)?,
        };

        Ok(())
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Download any mp3 files missing from the data directory
    Download,

    /// Rewrite the id3 tags of every mp3 file recorded as downloaded
    Tag,

    /// Rename post directories and mp3 files to match the current naming
    /// scheme, updating the database to match
    Rename {
        /// Report what would be renamed without touching anything
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
}
