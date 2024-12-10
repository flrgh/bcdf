mod bandcamp;
mod cli;
mod download;
mod feed;
mod metrics;
mod search;
mod spotify;
mod state;
mod tag;
mod types;
mod util;
mod http;

#[macro_use]
extern crate lazy_static;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::args();

    tracing_subscriber::fmt::init();

    let urls = match args.url {
        Some(url) => Vec::from([url]),
        None => feed::urls().await?,
    };

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
        metrics::inc(metrics::BlogPostsChecked, 1);

        let info = bandcamp::BlogInfo::try_from_url(&url).await?;

        let mut state = state::State::try_get_or_create(info, &args.download_to)?;

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

    for (metric, value) in metrics::summarize() {
        println!("{metric} => {value}");
    }

    Ok(())
}
