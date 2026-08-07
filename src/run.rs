use crate::{bandcamp, download, feed, http, metrics, spotify, tag};
use anyhow::Context;

#[derive(clap::Args, Debug, PartialEq)]
#[group(id = "run")]
pub(crate) struct Cli {
    /// Don't download anything
    #[arg(long, global = true, default_value_t = false)]
    no_download: bool,

    /// Don't create Spotify playlists
    #[arg(long, global = true, default_value_t = false)]
    no_spotify: bool,

    /// Scan only a single url
    #[arg(long, global = true)]
    url: Option<String>,

    /// Re-scan from the filesystem only
    #[arg(long, global = true, default_value_t = false)]
    rescan: bool,
}

impl Cli {
    pub(crate) async fn exec(self, store: &mut crate::store::Store) -> anyhow::Result<()> {
        let single_url = self.url.is_some();

        let urls = if self.rescan {
            let mut urls = Vec::new();
            if !self.no_spotify {
                urls.extend(store.posts_with_incomplete_spotify_data()?);
            }
            if !self.no_download {
                urls.extend(store.posts_with_incomplete_downloads()?);
            }
            urls.sort();
            urls.dedup();
            urls
        } else {
            match self.url {
                None => feed::urls(http::client()).await?,
                Some(url) => Vec::from([url]),
            }
        };

        if urls.is_empty() {
            tracing::info!("no posts to scrape, exiting");
            return Ok(());
        }

        let spotify = if self.no_spotify {
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

            let mut post = store.upsert_post(post, &scrape)?;
            metrics::inc(metrics::TracksSeen, post.tracks.len());

            if let Some(spotify) = &spotify {
                spotify.exec(store, &mut post).await?;
            }

            if !self.no_download {
                download::download(store, &mut post).await?;
                tag::tag(store, &post).await?;
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
}
