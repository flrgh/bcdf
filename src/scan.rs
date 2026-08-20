use anyhow::Context;

use crate::db::{
    ColumnTrait as _, Condition, CustomQueries as _, Db, EntityTrait as _, QueryFilter as _,
    TransactionTrait as _, col, entity, post::Urls as _, traits::Upsert,
};
use crate::{bandcamp, http, metrics, spotify};

/// Scrape new posts, download their mp3s, and update Spotify playlists
#[derive(clap::Args, Debug, PartialEq)]
#[group(id = "scan")]
pub(crate) struct Cli {
    /// Don't download anything
    #[arg(long, global = true, default_value_t = false)]
    no_download: bool,

    /// Don't create Spotify playlists
    #[arg(long, global = true, default_value_t = false)]
    no_playlists: bool,

    /// Don't search for any tracks on Spotify
    #[arg(long, global = true, default_value_t = false)]
    no_search: bool,

    /// alias for --no-playlists + --no-search
    #[arg(long, global = true, default_value_t = false)]
    no_spotify: bool,

    /// force a re-fetch of all posts
    #[arg(long, global = true, default_value_t = false)]
    force_scrape: bool,

    /// Scan only a single url
    #[arg(long, global = true)]
    url: Option<String>,

    /// Re-scan from the filesystem only
    #[arg(long, global = true, default_value_t = false)]
    rescan: bool,
}

async fn scrape_post(url: &str, client: &http::Client, db: &Db) -> anyhow::Result<()> {
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    tracing::info!("scraping post: {url}");

    let scrape_task = bandcamp::scrape(url, client)
        .await
        .context("fetching post html")?;

    let bandcamp::ScrapeModels {
        scrape,
        post,
        items,
    } = scrape_task
        .try_into_models()
        .context("parsing scraped post html result into models")?;

    if items.is_empty() {
        anyhow::bail!("scrape of {url} yielded no tracks");
    }

    let tx = db.begin().await?;

    scrape.upsert(&tx).await.context("scrape upsert")?;

    post.upsert(&tx).await.context("post upsert")?;

    let mut numbers = Vec::with_capacity(items.len());

    for (post_track, track, release, artist) in items.into_iter() {
        artist.upsert(&tx).await.context("artist upsert")?;
        release.upsert(&tx).await.context("release upsert")?;
        track.upsert(&tx).await.context("track upsert")?;
        let post_track = post_track.upsert(&tx).await.context("post_track upsert")?;
        numbers.push(post_track.post_track_number);
    }

    let orphaned = entity::PostTrack::delete_many()
        .filter(col::PostTrack::PostUrl.eq(url))
        .filter(col::PostTrack::PostTrackNumber.is_not_in(numbers))
        .exec(&tx)
        .await
        .context("deleting orphaned post tracks")?;

    if orphaned.rows_affected > 0 {
        tracing::info!(
            "deleted {} orphaned post track(s) for {url}",
            orphaned.rows_affected
        );
    }

    tx.commit().await?;

    Ok(())
}

impl Cli {
    pub(crate) async fn exec(mut self, app: &crate::App) -> anyhow::Result<()> {
        if self.no_spotify {
            self.no_search = true;
            self.no_playlists = true;
        }

        let urls = if self.rescan {
            let select = entity::Post::find();
            let mut cond = Condition::any();

            if !self.no_search {
                cond = cond.add(entity::Post::missing_spotify_tracks());
            }
            if !self.no_download {
                cond = cond.add(entity::Post::missing_track_downloads());
            }

            if cond.is_empty() {
                select.urls().all(&app.db).await?
            } else {
                select.filter(cond).urls().all(&app.db).await?
            }
        } else {
            match self.url {
                Some(url) => Vec::from([url]),
                None => bandcamp::urls_from_rss(http::client()).await?,
            }
        };

        if urls.is_empty() {
            tracing::info!("no posts to scrape, exiting");
            return Ok(());
        }

        let spotify = if self.no_playlists && self.no_search {
            None
        } else {
            Some(spotify::connect().await?)
        };

        let client = http::client();
        for url in urls {
            let mut post = if self.force_scrape {
                scrape_post(&url, client, &app.db).await?;
                app.db.get_post(&url).await?
            } else {
                match app.db.get_post(&url).await {
                    Ok(post) if post.scrape.is_some() => {
                        tracing::debug!("post already exists and has an existing scrape");
                        post
                    }
                    _ => {
                        if let Err(e) = scrape_post(&url, client, &app.db).await {
                            tracing::warn!("failed to scrape {url}: {e}");
                            continue;
                        }
                        app.db.get_post(&url).await?
                    }
                }
            };

            metrics::inc(metrics::BlogPostsSeen, 1);
            metrics::inc(metrics::TracksSeen, post.tracks.len());

            if let Some(spotify) = &spotify {
                if !self.no_search {
                    spotify.match_tracks(&app.db, &mut post).await?;
                }

                if !self.no_playlists {
                    spotify.update_post_playlist(&app.db, &mut post).await?;
                }
            }

            if !self.no_download {
                app.download(&mut post).await?;
                app.tag(&post).await?;
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
