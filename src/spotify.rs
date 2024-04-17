use std::cmp;

use anyhow::Context;
use chrono::TimeDelta;
use futures::stream::TryStreamExt;
use rspotify::model::{
    Country, IncludeExternal, Market, PlayableId, PlaylistId, SearchResult, SearchType, TrackId,
    UserId,
};
use rspotify::prelude::*;
use rspotify::{AuthCodeSpotify, Credentials};

use crate::search::{ResultScore, TrackMatcher};
use crate::state::State;
use crate::types;

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
    #[tracing::instrument]
    pub(crate) async fn get_or_create_playlist(&self, state: &mut State) -> anyhow::Result<()> {
        if !state.has_spotify_tracks() {
            tracing::debug!(
                title = state.blog_info.title,
                "no spotify tracks found for playlist"
            );
            return Ok(());
        }

        if state.spotify_playlist_id.is_some() {
            tracing::debug!(
                title = state.blog_info.title,
                "no action needed: playlist already created"
            );
            return Ok(());
        }

        let title = format!(
            "Bandcamp - {} - {}",
            state.blog_info.published.format("%Y-%m-%d"),
            &state.blog_info.title
        );

        tracing::debug!(name = &title, "searching for playlist");

        let mut res = self.spotify.current_user_playlists();
        while let Some(pl) = res.try_next().await.context("fetching user playlists")? {
            if pl.name == title {
                tracing::debug!(id = ?&pl.id, "found existing playlist");
                state.spotify_playlist_id = Some(pl.id.to_string());
                return Ok(());
            }
        }

        tracing::debug!("creating new playlist");
        let desc = format!("url: {}", state.blog_info.url);

        let pl = self
            .spotify
            .user_playlist_create(
                self.user.clone(),
                &title,
                Some(false),
                Some(false),
                Some(&desc),
            )
            .await
            .context("creating playlist")?;

        state.spotify_playlist_id = Some(pl.id.to_string());

        Ok(())
    }

    pub(crate) async fn search(&self, track: &mut types::Track) -> anyhow::Result<()> {
        if track.spotify_id.is_some() {
            return Ok(());
        }

        let query = format!("{} artist:{}", track.title, track.artist.name);

        let result = self
            .spotify
            .search(
                &query,
                SearchType::Track,
                Some(MARKET),
                Some(IncludeExternal::Audio),
                Some(10),
                None,
            )
            .await
            .with_context(|| format!("searching track: {}", track.title))?;

        let SearchResult::Tracks(tracks) = result else {
            anyhow::bail!("expected tracklist search result");
        };

        tracing::debug!(
            track = track.title,
            artist = track.artist.name,
            album = track.album.title,
            results = tracks.items.len(),
            "search results",
        );

        if tracks.items.is_empty() {
            return Ok(());
        }

        let mut tm = TrackMatcher::new(track);

        let mut best_score = None;
        let mut best = None;

        for result in tracks.items.iter() {
            let Some(score) = tm.score(result) else {
                continue;
            };

            if match best_score {
                Some(best_score) => best_score < score,
                None => true,
            } {
                best = Some(result);
                best_score = Some(score);
            }
        }

        let Some(best) = best else {
            tracing::debug!("no match for this track");
            return Ok(());
        };

        tracing::debug!(
            "Result track: {}, artist: {}, album: {}, # {}",
            best.name,
            best.artists[0].name,
            best.album.name,
            best.track_number
        );

        let id = best.id.clone().map(|id| id.to_string()).unwrap();
        tracing::info!("setting spotify id to {}", id);
        track.spotify_id = Some(id);

        Ok(())
    }

    pub(crate) async fn exec(&self, state: &mut State) -> anyhow::Result<()> {
        for track in state.tracks.iter_mut() {
            self.search(track).await.context("searching track")?;
        }

        self.get_or_create_playlist(state).await?;
        state.save()?;

        self.add_tracks_to_playlist(state).await?;
        state.save()?;

        Ok(())
    }

    async fn add_tracks_to_playlist(&self, state: &mut State) -> anyhow::Result<()> {
        if !state.needs_playlist_assignments() {
            return Ok(());
        }

        let Some(ref id) = state.spotify_playlist_id else {
            return Ok(());
        };

        let plid = PlaylistId::from_id_or_uri(id)?;

        let mut current_ids = std::collections::HashSet::new();
        let mut res = self
            .spotify
            .playlist_items(plid.clone(), None, Some(MARKET));

        while let Some(item) = res.try_next().await.context("fetching playlist track")? {
            let Some(track) = item.track else {
                continue;
            };

            let Some(track_id) = track.id() else {
                continue;
            };

            current_ids.insert(track_id.uri());
        }

        let mut add = vec![];
        for track in state.tracks.iter_mut() {
            let Some(ref spid) = track.spotify_id else {
                continue;
            };

            if current_ids.contains(spid) {
                continue;
            }

            if let Some(ref track_pl_id) = track.spotify_playlist_id {
                if *track_pl_id == *id {
                    continue;
                } else {
                    anyhow::bail!("that's weird... this track has a playlist id ({}), but it doesn't match the playlist we want to add it to ({})", track_pl_id, id);
                }
            }

            add.push(PlayableId::Track(TrackId::from_id_or_uri(spid)?));
            track.spotify_playlist_id = Some(id.to_owned());
        }

        if add.is_empty() {
            return Ok(());
        }

        let res = self
            .spotify
            .playlist_add_items(plid, add, None)
            .await
            .context("adding playlist items")?;
        dbg!(res);

        Ok(())
    }
}
