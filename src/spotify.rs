use anyhow::Context;
use futures::stream::TryStreamExt;
use rspotify::model::{
    Country, Market, PlayableId, PlaylistId, SearchResult, SearchType, TrackId, UserId,
};
use rspotify::prelude::*;
use rspotify::{AuthCodeSpotify, Credentials};

use crate::metrics;
use crate::search::TrackMatcher;
use crate::store::Store;
use crate::types::{self, BlogPost, SpotifyPlaylist};

#[derive(Debug)]
pub(crate) struct Client {
    spotify: AuthCodeSpotify,
    user: UserId<'static>,
}

const MARKET: Market = Market::Country(Country::UnitedStates);

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

impl Client {
    pub(crate) async fn get_or_create_playlist(
        &self,
        store: &Store,
        post: &mut BlogPost,
    ) -> anyhow::Result<()> {
        if !post.has_spotify_tracks() {
            tracing::debug!(title = post.title, "no spotify tracks found for playlist");
            return Ok(());
        }

        if post.spotify_playlist.is_some() {
            tracing::debug!(
                title = post.title,
                "no action needed: playlist already created"
            );
            return Ok(());
        }

        let name = format!(
            "Bandcamp - {} - {}",
            post.published.format("%Y-%m-%d"),
            post.title
        );

        tracing::debug!(name, "searching for playlist");

        let mut res = self.spotify.current_user_playlists();
        while let Some(pl) = res.try_next().await.context("fetching user playlists")? {
            if pl.name == name {
                tracing::debug!(id = ?&pl.id, "found existing playlist");
                let playlist = SpotifyPlaylist {
                    id: pl.id.to_string(),
                    name,
                };
                store.upsert_spotify_playlist(&post.url, &playlist)?;
                post.spotify_playlist = Some(playlist);
                return Ok(());
            }
        }

        tracing::debug!("creating new playlist");
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
            .context("creating playlist")?;

        let playlist = SpotifyPlaylist {
            id: pl.id.to_string(),
            name,
        };
        store.upsert_spotify_playlist(&post.url, &playlist)?;
        post.spotify_playlist = Some(playlist);

        metrics::inc(metrics::SpotifyPlaylistsCreated, 1);

        Ok(())
    }

    async fn do_search(
        &self,
        track_title: &str,
        artist: Option<&str>,
    ) -> anyhow::Result<Vec<rspotify::model::FullTrack>> {
        let query = match artist {
            Some(artist) => format!("track:{track_title} artist:{artist}"),
            None => format!("track:{track_title}"),
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
            .with_context(|| format!("searching track: {}", track_title))?;

        let SearchResult::Tracks(tracks) = result else {
            anyhow::bail!("unexpected track search results");
        };

        tracing::debug!(
            track = track_title,
            artist = artist,
            results = tracks.items.len(),
            "search results",
        );

        Ok(tracks.items)
    }

    pub(crate) async fn search(&self, track: &mut types::Track) -> anyhow::Result<()> {
        if track.spotify_id.is_some() {
            return Ok(());
        }

        let results = {
            let mut results = self
                .do_search(&track.title, Some(&track.artist.name))
                .await?;

            if results.len() < 5 && track.artist.name != track.album_artist.name {
                // also search by album artist if we didn't get enough results
                results.extend(
                    self.do_search(&track.title, Some(&track.album_artist.name))
                        .await?,
                );
            }

            results
        };

        if results.is_empty() {
            return Ok(());
        }

        let mut tm = TrackMatcher::new(track)?;

        let best = results
            .iter()
            .filter_map(|result| Some((tm.score(result)?, result)))
            .max_by(|(score_a, _), (score_b, _)| score_a.total_cmp(score_b));

        let Some((score, best)) = best else {
            tracing::info!(
                "no match for track('{}') out of {} results from Spotify",
                track.title,
                results.len()
            );
            return Ok(());
        };

        tracing::info!(
            "Result track: {}, artist: {}, album: {}, # {}, score: {}",
            best.name,
            best.artists[0].name,
            best.album.name,
            best.track_number,
            score
        );

        let Some(ref id) = best.id else {
            anyhow::bail!("Track: {best:?} does not have an ID");
        };

        let id = id.to_string();

        tracing::debug!("setting spotify id to {}", id);
        track.spotify_id = Some(id);
        track.spotify_match_score = Some(score);

        metrics::inc(metrics::TracksDiscoveredOnSpotify, 1);

        Ok(())
    }

    pub(crate) async fn exec(&self, store: &Store, post: &mut BlogPost) -> anyhow::Result<()> {
        let url = post.url.clone();

        for track in post.tracks.iter_mut() {
            if let Err(e) = self.search(track).await.context("searching track") {
                tracing::error!(?track, error = ?e, "failed to search track");
                metrics::inc(metrics::SpotifyErrors, 1);
            };

            match track.spotify_id {
                None => metrics::inc(metrics::TracksMissingFromSpotify, 1),
                Some(_) => store.update_track_spotify(&url, track)?,
            }
        }

        self.get_or_create_playlist(store, post).await?;
        self.add_tracks_to_playlist(store, post).await?;

        Ok(())
    }

    async fn add_tracks_to_playlist(
        &self,
        store: &Store,
        post: &mut BlogPost,
    ) -> anyhow::Result<()> {
        if !post.needs_playlist_assignments() {
            return Ok(());
        }

        let Some(playlist) = &post.spotify_playlist else {
            return Ok(());
        };

        let plid = PlaylistId::from_id_or_uri(&playlist.id)?;

        let mut current_ids = std::collections::HashSet::new();
        let mut res = self
            .spotify
            .playlist_items(plid.clone(), None, Some(MARKET));

        while let Some(item) = res.try_next().await.context("fetching playlist track")? {
            let Some(track) = item.item else {
                continue;
            };

            let Some(track_id) = track.id() else {
                continue;
            };

            current_ids.insert(track_id.uri());
        }

        let url = post.url.clone();

        let mut add = vec![];
        for track in post.tracks.iter_mut() {
            let Some(ref spid) = track.spotify_id else {
                continue;
            };

            if let Some(ref track_pl_id) = track.spotify_playlist_id {
                if *track_pl_id == *playlist.id {
                    continue;
                } else {
                    tracing::warn!("that's weird... this track has a playlist id ({}), but it doesn't match the playlist we want to add it to ({})", track_pl_id, playlist.id);
                }
            }

            if current_ids.contains(spid) {
                track.spotify_playlist_id = Some(plid.to_string());
                store.update_track_spotify(&url, track)?;
                continue;
            }

            add.push(PlayableId::Track(TrackId::from_id_or_uri(spid)?));
        }

        if !add.is_empty() {
            let num_tracks = add.len();

            self.spotify
                .playlist_add_items(plid.clone(), add, None)
                .await
                .context("adding playlist items")?;

            metrics::inc(metrics::TracksAddedToSpotifyPlaylist, num_tracks);

            // only recorded once Spotify has actually accepted them
            for track in post.tracks.iter_mut() {
                if track.spotify_id.is_some() && track.spotify_playlist_id.is_none() {
                    track.spotify_playlist_id = Some(plid.to_string());
                    store.update_track_spotify(&url, track)?;
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
    pub(crate) async fn exec(self, _store: &Store) -> anyhow::Result<()> {
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
                    PlaylistQuery::Id(playlist_id) => {
                        client.spotify.playlist(playlist_id, None, Some(MARKET))
                    }
                }
                .await?;

                let mut out = std::io::stdout().lock();
                serde_json::to_writer_pretty(&mut out, &serde_json::json!(playlist))?;
                println!()
            }

            Command::Search { title, artist } => {
                let client = connect().await?;
                let tracks = client.do_search(&title, artist.as_deref()).await?;
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
