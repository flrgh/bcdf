use anyhow::Context;
use chrono::prelude::*;
use clap::Subcommand;
use indexmap::IndexSet;
use rspotify::prelude::{Id, OAuthClient as _};

use crate::bandcamp::category::ALBUM_OF_THE_DAY;
use crate::db::playlist::{Kind, Status};
use crate::db::{
    ActiveValue::*, Apply as _, ColumnTrait as _, Condition, ConnectionTrait, CustomQueries as _,
    Db, EntityTrait as _, IntoActiveModel, JoinType, LimitIf as _, QueryFilter as _, QueryOrder,
    QuerySelect, QueryTrait, RelationTrait, TransactionSession, TransactionTrait, active_model,
    col, entity, model, post::Urls as _, rel, traits::Upsert, views,
};
use crate::types::DateTime;
use crate::{bandcamp, http, metrics, spotify};

#[derive(clap::Args, Debug)]
#[group(id = "scan")]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Posts(Posts),
    Tracks(Tracks),
    Playlists(Playlists),
}

impl Cli {
    pub(crate) async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        match self.command {
            Command::Posts(posts) => posts.exec(app).await,
            Command::Tracks(tracks) => tracks.exec(app).await,
            Command::Playlists(playlists) => playlists.exec(app).await,
        }
    }
}

#[derive(clap::Args, Debug)]
struct Opts {
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
}

#[derive(clap::Args, Debug)]
#[group(id = "scan-playlists")]
pub(crate) struct Playlists {
    #[arg(short = 'n', long, default_value_t = 10)]
    limit: u16,
}

fn start_of_month(dt: &DateTime) -> DateTime {
    dt.with_day(1)
        .unwrap()
        .with_hour(0)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap()
}

fn sub_month(dt: &DateTime) -> DateTime {
    let dt = start_of_month(dt);

    if dt.month() == 1 {
        dt.with_year(dt.year() - 1).unwrap().with_month(12).unwrap()
    } else {
        dt.with_month(dt.month() - 1).unwrap()
    }
}

fn add_month(dt: &DateTime) -> DateTime {
    let dt = start_of_month(dt);

    if dt.month() == 12 {
        dt.with_year(dt.year() + 1).unwrap().with_month(1).unwrap()
    } else {
        dt.with_month(dt.month() + 1).unwrap()
    }
}

async fn manage_album_of_the_day(
    db: &Db,
    spotify: &crate::spotify::Client,
    dt: &DateTime,
) -> anyhow::Result<bool> {
    let start = start_of_month(dt);
    let end = add_month(&start);

    let span = tracing::span!(
        tracing::Level::INFO,
        "album-of-the-day",
        month = start.format("%B %Y").to_string(),
    );
    let _guard = span.enter();

    let playlist = entity::Playlist::find()
        .filter(
            Condition::all()
                .add(col::Playlist::Kind.eq(Kind::Category))
                .add(col::Playlist::Category.eq(ALBUM_OF_THE_DAY))
                .add(col::Playlist::PeriodStart.eq(start))
                .add(col::Playlist::PeriodEnd.eq(end))
                .to_owned(),
        )
        .one(db)
        .await?;

    let post_tracks = entity::PostTrack::find()
        .inner_join(entity::Post)
        .filter(
            Condition::all()
                .add(col::Post::Category.eq(ALBUM_OF_THE_DAY))
                .add(col::Post::PublishedAt.gte(start))
                .add(col::Post::PublishedAt.lt(end))
                .add(col::Track::SpotifyId.is_not_null())
                .to_owned(),
        )
        .find_both_related(entity::Track)
        .order_by_asc(col::Post::PublishedAt)
        .order_by_asc(col::Post::Url)
        .order_by_asc(col::PostTrack::PostTrackNumber)
        .all(db)
        .await?;

    let matching: IndexSet<String> = post_tracks
        .iter()
        .filter_map(|elem| elem.1.spotify_id.clone())
        .collect();
    if matching.is_empty() {
        tracing::info!("no tracks with spotify matches found");
        return Ok(false);
    }

    let mut changed = false;

    if let Some(playlist) = playlist {
        tracing::info!("playlist exists already");
        let current_tracks = entity::PlaylistTrack::find()
            .filter(col::PlaylistTrack::PlaylistId.eq(&playlist.id))
            .inner_join(entity::Playlist)
            .inner_join(entity::Post)
            .find_both_related(entity::Track)
            .order_by_asc(col::Post::PublishedAt)
            .order_by_asc(col::Post::Url)
            .order_by_asc(col::PlaylistTrack::PostTrackNumber)
            .all(db)
            .await?
            .into_iter()
            .filter_map(|elem| elem.1.spotify_id)
            .collect::<IndexSet<String>>();

        if matching == current_tracks {
            tracing::info!("playlist is already up to date");
        } else {
            tracing::info!("playlist needs updates");
            changed = true;
            for diff in matching.difference(&current_tracks) {
                tracing::info!("difference: {diff}");
            }
        }
    } else {
        changed = true;
        let name = format!("bcdf daily albums: {}", start.format("%y%m"));
        tracing::info!("creating playlist {name}");

        let desc = format!(
            "Featured album-of-the-day tracks for {}",
            start.format("%B %Y")
        );

        let spl = spotify.create_playlist(&name, Some(desc.as_str())).await?;
        let id = spl.id.id();
        let now = chrono::Utc::now();
        let playlist = model::Playlist {
            id: id.to_owned(),
            name,
            kind: crate::db::playlist::Kind::Category,
            status: crate::db::playlist::Status::Active,
            created_at: now,
            updated_at: now,
            period_start: start,
            period_end: end,
            post_url: None,
            category: Some(ALBUM_OF_THE_DAY.to_string()),
        };

        let _ = entity::Playlist::insert(playlist.into_active_model())
            .exec(db)
            .await?;

        let items = matching
            .iter()
            .map(|id| rspotify::model::TrackId::from_id_or_uri(id).unwrap().into());

        let _ = spotify
            .spotify
            .playlist_add_items(spl.id.clone(), items, None)
            .await?;

        let rows: Vec<active_model::PlaylistTrack> = post_tracks
            .into_iter()
            .map(|(post_track, track)| active_model::PlaylistTrack {
                playlist_id: Set(id.to_owned()),
                post_url: Set(post_track.post_url),
                post_track_number: Set(post_track.post_track_number),
                track_id: Set(track.id),
            })
            .collect();

        entity::PlaylistTrack::insert_many(rows).exec(db).await?;
    }

    let playlists = entity::Playlist::find()
        .inner_join(entity::Post)
        .filter(
            Condition::all()
                .add(col::Playlist::Kind.eq("post"))
                .add(col::Playlist::Status.eq(Status::Active))
                .add(col::Post::Category.eq(ALBUM_OF_THE_DAY))
                .add(col::Post::PublishedAt.gte(start))
                .add(col::Post::PublishedAt.lt(end))
                .to_owned(),
        )
        .all(db)
        .await?;

    tracing::info!("found {} playlists to retire", playlists.len());
    for pl in playlists {
        changed = true;
        assert!(matches!(pl.kind, Kind::Post));
        spotify.retire_playlist(db, &pl).await?;
    }

    Ok(changed)
}

async fn update_album_of_the_day_playlists(db: &Db, limit: usize) -> anyhow::Result<()> {
    let Some((from, to)): Option<(DateTime, DateTime)> = entity::Post::find()
        .select_only()
        .column_as(col::Post::PublishedAt.min(), "min")
        .column_as(col::Post::PublishedAt.max(), "max")
        .into_tuple()
        .one(db)
        .await?
    else {
        tracing::warn!("no posts found??");
        return Ok(());
    };

    let spotify = spotify::connect().await?;

    let from = start_of_month(&from);
    let to = start_of_month(&to);

    let mut time = to;
    let mut limit = limit;
    while time > from && limit > 0 {
        if manage_album_of_the_day(db, &spotify, &time).await? {
            limit -= 1;
        }

        time = sub_month(&time);
    }

    Ok(())
}

impl Playlists {
    pub(crate) async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        update_album_of_the_day_playlists(&app.db, self.limit as usize).await?;
        Ok(())
    }
}

#[derive(clap::Args, Debug)]
#[group(id = "scan-tracks")]
pub(crate) struct Tracks {
    #[arg(long, global = true, default_value_t = false)]
    backfill_search: bool,

    #[arg(short = 'n', long, default_value_t = 100)]
    limit: u16,
}

impl Tracks {
    pub(crate) async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let spotify = spotify::connect().await?;

        let cond = if self.backfill_search {
            Condition::all()
                .add(col::Track::SpotifyId.is_not_null())
                .add(
                    Condition::any()
                        .add(col::Track::SpotifyMatchScore.is_null())
                        .add(col::TrackSearch::TrackId.is_null())
                        .to_owned(),
                )
                .to_owned()
        } else {
            Condition::all()
                .add(col::Track::SpotifyId.is_null())
                .add(col::TrackSearch::TrackId.is_null())
                .to_owned()
        };

        let tracks = views::PostTrackAll::select()
            .join(JoinType::FullOuterJoin, rel::Track::TrackSearch.def())
            .filter(cond)
            .order_by_desc(col::Post::PublishedAt)
            .limit(self.limit as u64)
            .into_partial_model::<views::PostTrackAll>()
            .all(&app.db)
            .await?;

        for row in tracks.into_iter() {
            let track = row.track;

            metrics::inc(metrics::TracksSeen, 1);
            println!("{} -> {}", track.track.id, track.track.title);

            let (id, score) = if self.backfill_search {
                spotify.re_score_matched_track(&app.db, &track).await?
            } else {
                match spotify.search(&app.db, &track).await? {
                    Some((id, score)) => (Some(id), Some(score)),
                    None => (None, None),
                }
            };

            if track.track.spotify_id != id || track.track.spotify_match_score != score {
                eprintln!("updating track spotify_id: {id:?}, score: {score:?}");
                entity::Track::update_spotify(&app.db, &track.track.id, id, score).await?;
            }
        }

        metrics::print();

        Ok(())
    }
}

/// Scrape new posts, download their mp3s, and update Spotify playlists
#[derive(clap::Args, Debug)]
#[group(id = "scan-posts")]
pub(crate) struct Posts {
    #[command(flatten)]
    opts: Opts,

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

impl Posts {
    pub(crate) async fn exec(mut self, app: &crate::App) -> anyhow::Result<()> {
        if self.opts.no_spotify {
            self.opts.no_search = true;
            self.opts.no_playlists = true;
        }

        let single_post = self.url.is_some();

        let urls = if self.rescan {
            let mut select = entity::Post::find()
                .left_join(entity::PlaylistTrack)
                .left_join(entity::Track)
                .join(
                    JoinType::FullOuterJoin,
                    crate::db::post::Relation::Playlist.def(),
                )
                .apply(|sel| self.filters.apply_timespec(sel, col::Post::PublishedAt))
                .limit_if(self.filters.limit())
                .order_by_desc(col::Post::PublishedAt);

            let mut cond = Condition::any().add(
                Condition::all()
                    .add(col::Track::SpotifyId.is_not_null())
                    .add(col::PlaylistTrack::PlaylistId.is_null())
                    .to_owned(),
            );

            let search = !self.opts.no_search;
            let download = !self.opts.no_download;

            if search {
                cond = cond.add(col::Track::SpotifyId.is_null());
            }

            if download {
                cond = cond.add(col::PostTrack::Filename.is_null());
            }

            if !cond.is_empty() {
                select = select.filter(cond);
            }

            // I cannot for the life of me seem to make `SELECT DISTINCT url`
            // work w/ sea-orm, so this'll have to do :shrug:
            let select = entity::Post::find().filter(
                col::Post::Url
                    .in_subquery(select.select_only().column(col::Post::Url).into_query()),
            );

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

        let spotify = if self.opts.no_playlists && self.opts.no_search {
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
                if !self.opts.no_search {
                    spotify.match_tracks(&app.db, &mut post).await?;
                }

                if !self.opts.no_playlists {
                    spotify.update_post_playlist(&app.db, &mut post).await?;
                }
            }

            if !self.opts.no_download {
                app.download(&mut post).await?;
                app.tag(&post).await?;
            }
        }

        metrics::print();

        Ok(())
    }
}
