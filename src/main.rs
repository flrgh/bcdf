mod bandcamp;
mod cli;
mod download;
mod feed;
mod http;
mod metrics;
mod search;
mod spotify;
mod store;
mod tag;
mod types;
mod util;

use anyhow::Context;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::args();

    tracing_subscriber::fmt::init();

    let single_url = args.url.is_some();

    let mut store = store::Store::open(&args.data_dir)?;

    let urls = if args.rescan {
        let mut urls = Vec::new();
        if !args.no_spotify {
            urls.extend(store.posts_with_incomplete_spotify_data()?);
        }
        if !args.no_download {
            urls.extend(store.posts_with_incomplete_downloads()?);
        }
        urls.sort();
        urls.dedup();
        urls
    } else {
        match args.url {
            None => feed::urls(http::client()).await?,
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
        metrics::inc(metrics::BlogPostsSeen, 1);

        let scraped = async {
            let scrape = bandcamp::scrape(&url, client).await?;
            let post = scrape.parse()?;
            anyhow::Ok((scrape, post))
        }
        .await
        .with_context(|| format!("scraping blog post from {url}"));

        let (scrape, post) = match scraped {
            Ok(scraped) => scraped,
            Err(e) if single_url => anyhow::bail!(e),
            Err(e) => {
                tracing::error!(?e, url);
                continue;
            }
        };

        let mut post = store.upsert_blog_post(post, &scrape)?;
        let dir = store.post_dir(&post);
        metrics::inc(metrics::TracksSeen, post.tracks.len());

        if let Some(spotify) = &spotify {
            spotify.exec(&store, &mut post).await?;
        }

        if !args.no_download {
            download::download(&dir, &post.tracks).await;
            tag::tag(&dir, &post.tracks).await?;
        }
    }

    for (metric, value) in metrics::summarize() {
        println!(
            "{metric:width$} => {value}",
            width = &metrics::MAX_STRING_WIDTH
        );
    }

    Ok(())
}
