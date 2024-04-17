mod bandcamp;
mod download;
mod feed;
mod search;
mod spotify;
mod state;
mod tag;
mod types;
mod util;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Base directory for storing downloaded content
    #[arg(long, default_value_t = crate::state::OUT_DIR.to_string())]
    download_to: String,

    /// Don't download anything
    #[arg(long, default_value_t = false)]
    no_download: bool,

    /// Don't create Spotify playlists
    #[arg(long, default_value_t = false)]
    no_spotify: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

    let urls = feed::urls().await?;

    if urls.is_empty() {
        tracing::info!("no posts to scrape, exiting");
        return Ok(());
    }

    let spotify = if args.no_spotify {
        None
    } else {
        Some(spotify::connect().await?)
    };

    for url in urls {
        tracing::info!("scanning post: {url}");

        let info = bandcamp::BlogInfo::try_from_url(&url).await?;

        let mut state = state::State::try_get_or_create(info)?;

        if let Some(spotify) = &spotify {
            spotify.exec(&mut state).await?;
            state.save()?;
        }

        if !args.no_download {
            download::download(&state).await;
            state.save()?;

            tag::tag(&state).await?;
        }
    }

    Ok(())
}
