use std::collections::HashSet;
use std::ops::Deref as _;

use anyhow::Context;
use futures::stream::TryStreamExt;
use rspotify::{
    AuthCodeSpotify, ClientError, Credentials,
    http::HttpError,
    model::{Country, Market, PlaylistId, SearchResult, SearchType, TrackId, UserId},
    prelude::*,
};

use crate::db::{CustomQueries as _, Db, entity, full, model, traits::SpotifyPlaylistId};
use crate::metrics;
use crate::search::{MatchResult, TrackArtists, TrackMatcher, TrackTitle};

#[derive(Debug)]
pub(crate) struct Client {
    spotify: AuthCodeSpotify,
    user: UserId<'static>,
}

const MARKET: Market = Market::Country(Country::UnitedStates);

trait Is404 {
    fn is_404(&self) -> bool;
}

impl Is404 for ClientError {
    fn is_404(&self) -> bool {
        if let ClientError::Http(err) = self
            && let HttpError::StatusCode(res) = err.deref()
        {
            return res.status().as_u16() == 404;
        };

        false
    }
}

async fn inspect_client_error(e: ClientError) -> anyhow::Error {
    match e {
        ClientError::Http(err) => match *err {
            HttpError::StatusCode(res) => {
                let span = tracing::span!(tracing::Level::ERROR, "rspotify response");
                let _guard = span.enter();

                tracing::error!(
                    url = %res.url(),
                    status = %res.status().as_u16(),
                    reason = %(res.status().canonical_reason().unwrap_or("unknown")),
                    "request returned non-2xx status code",
                );

                for (name, value) in res.headers().into_iter() {
                    if let Ok(s) = value.to_str() {
                        tracing::debug!(header = %name, value = s);
                    } else {
                        tracing::debug!(header = %name, value = ?value);
                    }
                }

                if let Ok(bytes) = res.bytes().await {
                    if let Ok(pretty) = serde_json::from_slice::<serde_json::Value>(&bytes)
                        .and_then(|value| serde_json::to_string_pretty(&value))
                    {
                        tracing::debug!("response.json" = pretty);
                    } else {
                        tracing::debug!("response.bytes" = ?bytes);
                    }
                };

                anyhow::anyhow!("request returned non-2xx status code")
            }
            HttpError::Client(error) => anyhow::anyhow!(error),
        },
        _ => anyhow::anyhow!(e),
    }
}

trait InspectClientError {
    type Output;
    async fn inspect_client_error(self) -> Self::Output;
}

impl<T: Send + Sync + Sized> InspectClientError for anyhow::Result<T, ClientError> {
    type Output = anyhow::Result<T>;

    async fn inspect_client_error(self) -> anyhow::Result<T> {
        match self {
            Ok(t) => Ok(t),
            Err(e) => Err(inspect_client_error(e).await),
        }
    }
}

pub(crate) async fn connect() -> anyhow::Result<Client> {
    let config = rspotify::Config {
        token_cached: true,
        token_refreshing: true,
        ..Default::default()
    };

    let Some(creds) = Credentials::from_env() else {
        anyhow::bail!("failed reading credentials from env");
    };

    let scopes = rspotify::scopes!(
        "playlist-read-private",
        "playlist-read-collaborative",
        "playlist-modify-private",
        "playlist-modify-public"
    );

    let Some(oauth) = rspotify::OAuth::from_env(scopes) else {
        anyhow::bail!("failed setting up OAuth");
    };

    let spotify = AuthCodeSpotify::with_config(creds, oauth, config);
    let url = spotify
        .get_authorize_url(false)
        .context("getting Spotify auth url")?;
    spotify
        .prompt_for_token(&url)
        .await
        .context("prompting for Spotify token")?;

    let user = spotify.current_user().await?.id.into_static();

    spotify.write_token_cache().await?;

    Ok(Client { spotify, user })
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
struct TrackQuery {
    title: String,
    artist: Option<String>,
}

impl From<&str> for TrackQuery {
    fn from(value: &str) -> Self {
        Self {
            title: value.to_owned(),
            artist: None,
        }
    }
}

impl From<&String> for TrackQuery {
    fn from(value: &String) -> Self {
        Self {
            title: value.to_owned(),
            artist: None,
        }
    }
}

impl From<String> for TrackQuery {
    fn from(value: String) -> Self {
        Self {
            title: value,
            artist: None,
        }
    }
}

impl<T1, T2> From<(T1, T2)> for TrackQuery
where
    T1: AsRef<str>,
    T2: AsRef<str>,
{
    fn from(value: (T1, T2)) -> Self {
        let (title, artist) = (value.0.as_ref(), value.1.as_ref());
        Self {
            title: title.to_owned(),
            artist: Some(artist.to_owned()),
        }
    }
}

impl Client {
    pub(crate) async fn get_playlist<T: SpotifyPlaylistId>(
        &self,
        t: &T,
    ) -> anyhow::Result<Option<rspotify::model::FullPlaylist>> {
        let id = t.playlist_id();
        match self.spotify.playlist(id, None, Some(MARKET)).await {
            Ok(pl) => Ok(Some(pl)),
            Err(e) if e.is_404() => Ok(None),
            Err(e) => Err(e).inspect_client_error().await,
        }
    }

    pub(crate) async fn create_playlist(
        &self,
        db: &Db,
        post: &model::Post,
    ) -> anyhow::Result<model::Playlist> {
        let name = post.playlist_name();

        tracing::debug!("creating playlist: {name}");

        let pl = self
            .spotify
            .user_playlist_create(
                self.user.clone(),
                &name,
                Some(false),
                Some(false),
                Some(&post.url),
            )
            .await
            .inspect_client_error()
            .await?;

        let created = db.upsert_playlist(&post.url, pl.id.id(), &pl.name).await?;

        metrics::inc(metrics::SpotifyPlaylistsCreated, 1);

        Ok(created)
    }

    async fn do_search(
        &self,
        query: TrackQuery,
    ) -> anyhow::Result<Vec<rspotify::model::FullTrack>> {
        let TrackQuery { title, artist } = query;

        let query = match artist {
            Some(artist) => format!("track:{title} artist:{artist}"),
            None => format!("track:{title}"),
        };

        metrics::inc(metrics::SpotifyTrackSearchQueries, 1);

        let result = self
            .spotify
            .search(
                &query,
                SearchType::Track,
                Some(MARKET),
                None,
                Some(10),
                None,
            )
            .await
            .inspect_client_error()
            .await
            .with_context(|| format!("searching track: {}", title))?;

        let SearchResult::Tracks(tracks) = result else {
            anyhow::bail!("unexpected track search results");
        };

        tracing::debug!(
            query = %query,
            results = tracks.items.len(),
            "spotify search",
        );

        Ok(tracks.items)
    }

    #[tracing::instrument(
        skip_all,
        fields(
            track = %track.track.title,
            artist = %track.artist.name,
            album = %track.release.title,
            secs = track.track.duration,
        )
    )]
    pub(crate) async fn search(
        &self,
        track: &full::Track,
    ) -> anyhow::Result<Option<(String, f64)>> {
        fn track_queries(track: &model::Track, artist: &model::Artist) -> Vec<TrackQuery> {
            let title = TrackTitle::from_str(&track.title);
            let artists = TrackArtists::from_str(&artist.name);

            let mut queries = Vec::with_capacity(4);
            let mut seen = HashSet::new();
            let mut push = |elem: TrackQuery| {
                if !seen.contains(&elem) {
                    seen.insert(elem.clone());
                    queries.push(elem);
                }
            };

            if let Some(credited_artist) = &track.credited_artist {
                push((title.core(), credited_artist).into());
            }

            push((title.core(), artists.primary()).into());
            push((&track.title, artists.primary()).into());
            push(title.core().into());

            queries
        }

        let tm = TrackMatcher::from_track(track);
        let mut seen = 0;
        let mut nearest: Option<MatchResult> = None;

        let queries = track_queries(&track.track, &track.artist);
        for (attempt, query) in queries.into_iter().enumerate() {
            let results = self.do_search(query).await?;
            let num_results = results.len();
            seen += num_results;

            let span = tracing::debug_span!("attempt", n = attempt, results = num_results);
            let _guard = span.enter();

            let mut matched = Vec::new();
            for result in results {
                let Some(id) = &result.id else {
                    tracing::debug!("skipping search result with no track id");
                    continue;
                };

                let res = tm.score(&result);

                if let Some(score) = res.matched() {
                    let id = id.clone();
                    matched.push((score, result, id));
                    continue;
                }

                if nearest
                    .as_ref()
                    .is_none_or(|prev| res.score() > prev.score())
                {
                    nearest.replace(res);
                }
            }

            let best = matched
                .into_iter()
                .max_by(|(score_a, _, _), (score_b, _, _)| score_a.total_cmp(score_b));

            let Some((score, best, id)) = best else {
                tracing::debug!(candidates = num_results, "no candidate was a match");
                continue;
            };

            let id = id.to_string();

            tracing::info!(
                id = %id,
                name = %best.name,
                artist = %best.artists[0].name,
                album = %best.album.name,
                number = best.track_number,
                secs = best.duration.num_milliseconds() as f64 / 1000.0,
                score = %format_args!("{score:.1}"),
                attempt,
                "matched",
            );

            metrics::inc(metrics::TracksDiscoveredOnSpotify, 1);

            return Ok(Some((id, score)));
        }

        match nearest {
            Some(nearest) => tracing::info!(results = seen, closest = %nearest, "no match"),
            None => tracing::info!(results = seen, "no match: Spotify returned nothing"),
        }

        Ok(None)
    }

    pub(crate) async fn match_tracks(&self, db: &Db, data: &mut full::Post) -> anyhow::Result<()> {
        for post_track in data.tracks.iter_mut() {
            let track = &mut post_track.track;
            if track.track.spotify_id.is_some() {
                continue;
            }

            match self.search(track).await.context("searching track") {
                Err(e) => {
                    tracing::error!(
                        track = %track.track.title,
                        artist = %track.artist.name,
                        error = ?e,
                        "failed to search track",
                    );
                    metrics::inc(metrics::SpotifyErrors, 1);
                }
                Ok(None) => {
                    metrics::inc(metrics::TracksMissingFromSpotify, 1);
                }
                Ok(Some((id, score))) => {
                    track.track.spotify_id = Some(id);
                    track.track.spotify_match_score = Some(score);
                    track.track = db.update_track(&track.track).await?;
                }
            };
        }

        Ok(())
    }

    async fn delete_playlist(
        &self,
        db: &Db,
        pl: model::Playlist,
        tracks: &mut [full::PostTrack],
    ) -> anyhow::Result<()> {
        for track in tracks.iter_mut() {
            track.post_track.spotify_playlist_id = None;

            match db.update_post_track(&track.post_track).await {
                Ok(pt) => track.post_track = pt,
                Err(e) => {
                    tracing::warn!("failed updating post track: {e}");
                }
            }
        }

        let db_res = entity::Playlist::delete_by_post_url(&pl.post_url)
            .exec(db)
            .await
            .context("deleting playlist from the database");

        if let Err(e) = self
            .spotify
            .library_remove([pl.library_id()])
            .await
            .inspect_client_error()
            .await
        {
            tracing::error!(
                "failed deleting post {} playlist ({}) from spotify: {}",
                &pl.post_url,
                &pl.id,
                e,
            );
        };

        let _ = db_res?;

        Ok(())
    }

    pub(crate) async fn set_playlist_user_deleted(
        &self,
        db: &Db,
        pl: &mut model::Playlist,
    ) -> anyhow::Result<()> {
        use crate::db::{ColumnTrait, EntityTrait, Expr, QueryFilter, col, entity};
        let _ = entity::Playlist::update_many()
            .col_expr(col::Playlist::UserDeleted, Expr::value(true))
            .filter(col::Playlist::Id.eq(&pl.id))
            .exec(db)
            .await?;

        pl.user_deleted = true;

        Ok(())
    }

    pub(crate) async fn update_post_playlist(
        &self,
        db: &Db,
        data: &mut full::Post,
    ) -> anyhow::Result<()> {
        let has_spotify_tracks = data
            .tracks
            .iter()
            .any(|t| t.track.track.spotify_id.is_some());

        if !has_spotify_tracks {
            let Some(pl) = data.playlist.take() else {
                return Ok(());
            };

            tracing::info!(
                "post {} has no spotify tracks--deleting orphaned playlist {}",
                &data.post.url,
                &pl.id
            );

            return self.delete_playlist(db, pl, &mut data.tracks).await;
        }

        let pl = match &mut data.playlist {
            Some(pl) if pl.user_deleted => {
                tracing::info!("skipping deleted playlist {}", pl.id);
                return Ok(());
            }
            Some(pl) => {
                let exp_name = data.post.playlist_name();

                if pl.name != exp_name {
                    tracing::info!(
                        "renaming post {} playlist '{}' -> '{}'",
                        &data.post.url,
                        &pl.name,
                        &exp_name
                    );
                    if let Err(e) = self
                        .spotify
                        .playlist_change_detail(
                            pl.spotify_playlist_id(),
                            Some(&exp_name),
                            None,
                            None,
                            None,
                        )
                        .await
                    {
                        if e.is_404() {
                            tracing::info!(
                                "post {} playlist ({}) has been deleted",
                                &data.post.url,
                                &pl.id
                            );
                            self.set_playlist_user_deleted(db, pl).await?;
                            return Ok(());
                        }

                        return Err(inspect_client_error(e).await);
                    }

                    let _ = db.upsert_playlist(&pl.post_url, &pl.id, &exp_name).await?;
                    pl.name = exp_name;
                }

                pl
            }
            None => {
                let pl = self.create_playlist(db, &data.post).await?;
                data.playlist.insert(pl)
            }
        };

        self.update_playlist_tracks(db, pl, &mut data.tracks)
            .await?;

        Ok(())
    }

    async fn update_playlist_tracks(
        &self,
        db: &Db,
        playlist: &mut model::Playlist,
        tracks: &mut [full::PostTrack],
    ) -> anyhow::Result<()> {
        let post_url = &playlist.post_url;
        let plid = playlist.spotify_playlist_id().into_static();

        let local_items: Vec<_> = tracks
            .iter()
            .filter_map(|t| t.spotify_playable_id())
            .collect();

        let remote_items = {
            let mut remote_items = vec![];

            let mut playlist_tracks = self
                .spotify
                .playlist_items(plid.clone(), None, Some(MARKET));

            loop {
                match playlist_tracks.try_next().await {
                    Ok(Some(item)) => {
                        let Some(item) = item.item else {
                            continue;
                        };

                        let Some(id) = item.id() else {
                            continue;
                        };

                        remote_items.push(id.into_static());
                    }
                    Ok(None) => break,
                    Err(e) if e.is_404() => {
                        tracing::info!("playlist ({plid}) for post {post_url} has been deleted");
                        self.set_playlist_user_deleted(db, playlist).await?;
                        return Ok(());
                    }
                    Err(e) => anyhow::bail!(e),
                }
            }

            remote_items
        };

        if local_items == remote_items {
            tracing::debug!("no updates needed to post {post_url} playlist ({plid})",);
            return Ok(());
        }

        let num_tracks = local_items.len();

        tracing::info!("updating post {post_url} playlist ({plid}) with {num_tracks} items");

        if let Err(e) = self
            .spotify
            .playlist_replace_items(plid.clone(), local_items)
            .await
        {
            for track in tracks.iter_mut() {
                track.post_track.spotify_playlist_id = None;
                if let Ok(pt) = db.update_post_track(&track.post_track).await {
                    track.post_track = pt;
                }
            }

            anyhow::bail!("failed updating post {post_url} playlist {plid}: {e}",);
        };

        metrics::inc(metrics::TracksAddedToSpotifyPlaylist, num_tracks);

        for track in tracks.iter_mut() {
            track.post_track.spotify_playlist_id = if track.track.track.spotify_id.is_some() {
                Some(playlist.id.to_string())
            } else {
                None
            };

            match db.update_post_track(&track.post_track).await {
                Ok(pt) => track.post_track = pt,
                Err(e) => {
                    tracing::warn!("failed updating post track: {e}");
                }
            }
        }

        Ok(())
    }
}

/// Spotify management and debug actions
#[derive(clap::Args, Debug)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub(crate) async fn exec(self) -> anyhow::Result<()> {
        match self.command {
            Command::Track { id } => {
                let client = connect().await?;

                match client.spotify.track(id, Some(MARKET)).await {
                    Ok(track) => {
                        let mut out = std::io::stdout().lock();
                        serde_json::to_writer_pretty(&mut out, &serde_json::json!(track))?;
                        println!()
                    }
                    Err(e) => anyhow::bail!(e),
                }
            }

            Command::Playlist { query } => {
                let client = connect().await?;
                let playlist = match query {
                    PlaylistQuery::Id(id) => client
                        .get_playlist(&id)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("playlist not found"))?,
                };

                let mut out = std::io::stdout().lock();
                serde_json::to_writer_pretty(&mut out, &serde_json::json!(playlist))?;
                println!()
            }

            Command::Search { title, artist } => {
                let client = connect().await?;
                let tracks = client.do_search(TrackQuery { title, artist }).await?;
                let mut out = std::io::stdout().lock();
                serde_json::to_writer_pretty(&mut out, &serde_json::json!(tracks))?;
                println!()
            }
        };
        Ok(())
    }
}

fn track_id(input: &str) -> anyhow::Result<TrackId<'static>> {
    let id = TrackId::from_id_or_uri(input)?;
    Ok(id.into_static())
}

#[derive(Debug, Clone)]
pub(crate) enum PlaylistQuery {
    Id(PlaylistId<'static>),
}

impl std::str::FromStr for PlaylistQuery {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let id = PlaylistId::from_id_or_uri(s)?;
        Ok(Self::Id(id.into_static()))
    }
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Fetch and print a Spotify track by its ID
    Track {
        #[arg(value_name = "TRACK_ID", value_parser = track_id)]
        id: TrackId<'static>,
    },

    /// Fetch and print a Spotify playlist by its ID
    Playlist {
        #[arg(value_name = "ID_OR_NAME")]
        query: PlaylistQuery,
    },

    /// Search a track on Spotify and print the results
    Search {
        #[arg(value_name = "TRACK_TITLE")]
        title: String,

        #[arg(long, global = true, value_name = "ARTIST")]
        artist: Option<String>,
    },
}
