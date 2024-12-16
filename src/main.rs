mod bandcamp;
mod cli;
mod download;
mod feed;
mod http;
mod metrics;
mod search;
mod spotify;
mod state;
mod tag;
mod types;
mod util;

use anyhow::Context;

#[macro_use]
extern crate lazy_static;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::args();

    tracing_subscriber::fmt::init();

    let single_url = args.url.is_some();

    let urls = if args.rescan {
        state::blog_urls(&args)?
    } else {
        match args.url {
            None => feed::urls().await?,
            Some(url) => Vec::from([url]),
        }
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

    let client = http::client();
    for url in urls {
        tracing::info!("scanning post: {url}");
        metrics::inc(metrics::BlogPostsChecked, 1);

        let info = bandcamp::BlogInfo::try_from_url(&url, &client)
            .await
            .with_context(|| format!("fetching blog info from {url}"));

        let info = match info {
            Ok(info) => info,
            Err(e) if single_url => anyhow::bail!(e),
            Err(e) => {
                tracing::error!(?e, url);
                continue;
            }
        };

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
