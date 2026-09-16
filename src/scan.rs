use anyhow::Context;

use crate::db::{Apply as _, LimitIf as _};
use crate::db::{
    ColumnTrait as _, Condition, ConnectionTrait, CustomQueries as _, EntityTrait as _,
    QueryFilter as _, TransactionSession, TransactionTrait, col, entity, model, post::Urls as _,
    traits::Upsert, views,
};
use crate::{bandcamp, http, metrics, spotify};

/// Scrape new posts, download their mp3s, and update Spotify playlists
#[derive(clap::Args, Debug)]
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

    /// re-scrape post html even if existing scrape data exists
    #[arg(long, global = true, default_value_t = false)]
    force_scrape: bool,

    /// re-parse post data/track from scrape data even if a post already exists
    #[arg(long, global = true, default_value_t = false)]
    re_parse: bool,

    /// Scan only a single url
    #[arg(long, global = true)]
    url: Option<String>,

    /// Re-scan from the filesystem only
    #[arg(long, global = true, default_value_t = false)]
    rescan: bool,

    #[command(flatten)]
    filters: crate::list::Filters,
}

impl model::Scrape {
    async fn upsert_post_items<C>(self, db: &C) -> anyhow::Result<views::PostItems>
    where
        C: ConnectionTrait + TransactionTrait,
    {
        let url = &self.url;

        let bandcamp::PostModels { post, items } = self
            .try_as_post_models()
            .context("deriving post models from scraped player data")?;

        if items.is_empty() {
            tracing::warn!("scrape of {url} yielded no tracks");
        }

        let tx = db.begin().await?;

        let post = post.upsert(&tx).await.context("post upsert")?;

        let old = post.tracks(&tx).await?;
        let by_num = |num: u32| -> Option<&model::PostTrack> {
            old.iter().find(|pt| pt.post_track_number == num)
        };
        let by_id = |id: &str| {
            old.iter()
                .filter(|pt| pt.track_id.eq(id))
                .collect::<Vec<_>>()
        };

        let mut numbers = Vec::with_capacity(items.len());

        for (mut post_track, track, release, artist) in items.into_iter() {
            let id = post_track.track_id.as_ref();
            let num = *post_track.post_track_number.as_ref();
            numbers.push(num);

            let span = tracing::debug_span!("post_track", post_track_number = num, track_id = id);
            let _guard = span.enter();

            let others = by_id(id);
            match others.get(..) {
                None | Some(&[]) => {
                    tracing::debug!("adding a new post track");
                }
                Some(&[one]) => {
                    tracing::debug!("updating an existing post track");
                    if one.post_track_number != num {
                        tracing::warn!(
                            "track number updating from {} to {num}",
                            one.post_track_number
                        );
                    }
                    post_track.filename.set_ne(one.filename.clone());
                    post_track
                        .spotify_playlist_id
                        .set_ne(one.spotify_playlist_id.clone());
                }
                Some(rest) => {
                    tracing::warn!("{} existing post tracks with this id", rest.len());

                    for other in rest {
                        match (other.filename.as_ref(), post_track.filename.as_option()) {
                            (Some(other), None) => {
                                tracing::debug!("adopting existing filename: {other}");
                                post_track.filename =
                                    sea_orm::ActiveValue::Set(Some(other.clone()));
                            }
                            (Some(other), Some(mine)) if other != mine => anyhow::bail!(
                                "conflicting values for track filename: {mine}, {other}"
                            ),
                            _ => {}
                        }

                        match (
                            other.spotify_playlist_id.as_ref(),
                            post_track.spotify_playlist_id.as_option(),
                        ) {
                            (Some(other), None) => {
                                tracing::debug!("adopting existing spotify_playlist_id: {other}");
                                post_track.spotify_playlist_id =
                                    sea_orm::ActiveValue::Set(Some(other.clone()));
                            }
                            (Some(other), Some(mine)) if other != mine => anyhow::bail!(
                                "conflicting values for track spotify_playlist_id: {mine}, {other}"
                            ),
                            _ => {}
                        }
                    }
                }
            }

            if let Some(other) = by_num(num)
                && &other.track_id != id
            {
                tracing::warn!(
                    "another post track exists with a different id: {}",
                    &other.track_id
                );
            };

            artist.upsert(&tx).await.context("artist upsert")?;
            release.upsert(&tx).await.context("release upsert")?;
            track.upsert(&tx).await.context("track upsert")?;

            post_track.upsert(&tx).await.context("post_track upsert")?;
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

        db.get_post(url).await
    }
}

impl crate::App {
    pub async fn scrape(
        &self,
        url: &str,
        client: &http::Client,
    ) -> anyhow::Result<views::PostItems> {
        let scrape = bandcamp::scrape(url, client).await?;

        let tx = self.db.begin().await?;

        let scrape = scrape.upsert(&tx).await?;

        let post = scrape.upsert_post_items(&tx).await?;

        tx.commit().await?;

        Ok(post)
    }
}

impl Cli {
    pub(crate) async fn exec(mut self, app: &crate::App) -> anyhow::Result<()> {
        if self.no_spotify {
            self.no_search = true;
            self.no_playlists = true;
        }

        let single_post = self.url.is_some();

        let urls = if self.rescan {
            let mut select = entity::Post::find()
                .apply(|sel| self.filters.apply_timespec(sel, col::Post::PublishedAt))
                .limit_if(self.filters.limit());

            let mut cond = Condition::any();

            let search = !self.no_search;
            let download = !self.no_download;

            if search {
                cond = cond.add(entity::Post::missing_spotify_tracks());
            }

            if download {
                cond = cond.add(entity::Post::missing_track_downloads());
            }

            if !cond.is_empty() {
                select = select.filter(cond);
            }

            select.urls().all(&app.db).await?
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
            let found = app.db.get_post(&url).await.ok();

            let mut post = match (found, self.force_scrape, self.re_parse) {
                (Some(post), false, false) => {
                    tracing::debug!("using existing post data for {url}");
                    post
                }

                (_, true, _) => {
                    tracing::info!("re-scraping post {url}");

                    // don't continue on error if an explicit re-scrape was requested
                    app.scrape(&url, client).await?
                }

                (Some(post), false, true) => {
                    let scrape = entity::Scrape::find_by_id(&post.post.url)
                        .one(&app.db)
                        .await?;

                    match scrape {
                        Some(scrape) => {
                            tracing::info!("re-deriving post {url} from scrape data");
                            scrape.upsert_post_items(&app.db).await?
                        }
                        None => app.scrape(&url, client).await?,
                    }
                }

                (None, _, _) => match (app.scrape(&url, client).await, single_post) {
                    (Ok(post), _) => post,
                    (Err(e), false) => {
                        tracing::warn!("failed to scrape {url}: {e}");
                        continue;
                    }
                    (Err(e), true) => anyhow::bail!(e),
                },
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
