use crate::{download, tag};
use anyhow::Context;
use clap::Subcommand;

/// Manage downloaded mp3 files
#[derive(clap::Args, Debug)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub(crate) async fn exec(self, store: &crate::store::Store) -> anyhow::Result<()> {
        match self.command {
            Command::Download => {
                for post in store.list_posts()? {
                    let dir = store.post_dir(&post);
                    std::fs::create_dir_all(&dir)
                        .with_context(|| format!("creating post directory {dir:?}"))?;
                    download::download(&dir, &post.tracks).await;
                }
            }
            Command::Tag => {
                for post in store.list_posts()? {
                    tag::tag(&store.post_dir(&post), &post.tracks).await?;
                }
            }
        };

        Ok(())
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Download any mp3 files missing from the data directory
    Download,

    /// Rewrite the id3 tags of every downloaded mp3 file
    Tag,
}
