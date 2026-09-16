use rss::Channel;
use scraper::{Html, Selector};
use sea_orm::ActiveValue::{NotSet, Set};
use serde::Deserialize;
use serde_json::Value;
use std::{borrow::Cow, sync::LazyLock};

use crate::compress;
use crate::db::{active_model, model, release::Type as ReleaseType};
use crate::http;
use crate::types::DateTime;

pub(crate) const DAILY_BASE_URL: &str = "https://daily.bandcamp.com";
pub(crate) const FEED_URL: &str = "https://daily.bandcamp.com/feed/";

#[derive(Debug, Clone)]
struct HtmlSelector(Selector);

impl HtmlSelector {
    fn try_new(s: &str) -> anyhow::Result<Self> {
        match Selector::parse(s) {
            Ok(selector) => Ok(Self(selector)),
            Err(e) => {
                anyhow::bail!("failed parsing CSS selector from '{s}': {e}");
            }
        }
    }

    fn select<'a, 'b>(&'a self, doc: &'b Html) -> scraper::html::Select<'b, 'a> {
        doc.select(&self.0)
    }
}

macro_rules! selector {
    ($name:ident, $s:expr) => {
        selector!($name, $s, HtmlSelector::try_new);
    };
    ($name:ident, $s:expr, $fn:path) => {
        static $name: LazyLock<HtmlSelector> =
            LazyLock::new(|| $fn($s).expect("invalid CSS selector"));
    };
}

selector!(DAILY_ARTICLE, "#p-daily-article");
selector!(POST_META, "head meta");

#[derive(Debug, PartialEq, Clone, Deserialize)]
pub(crate) struct TrackInfo {
    pub(crate) artist: String,

    pub(crate) audio_track_duration: f64,
    pub(crate) track_number: usize,
    pub(crate) track_title: String,
    pub(crate) audio_url: std::collections::BTreeMap<String, String>,

    pub(crate) album_id: Option<u64>,
    pub(crate) track_id: u64,
}

#[derive(Debug, PartialEq, Clone, Deserialize)]
pub(crate) struct PlayerData {
    pub(crate) title: String,
    pub(crate) tracklist: Vec<TrackInfo>,
    pub(crate) featured_track_number: usize,

    pub(crate) band_name: String,
    pub(crate) band_id: u64,
    pub(crate) band_url: String,
    pub(crate) tralbum_url: String,
    pub(crate) parent_tralbum_id: u64,
    pub(crate) parent_tralbum_type: char,
}

impl PlayerData {
    pub(crate) fn try_into_models(
        self,
    ) -> anyhow::Result<
        Option<(
            active_model::Track,
            active_model::Release,
            active_model::Artist,
        )>,
    > {
        let PlayerData {
            title,
            tracklist,
            featured_track_number,
            band_name,
            band_id,
            band_url,
            tralbum_url,
            parent_tralbum_id,
            parent_tralbum_type,
        } = self;

        let Some(ti) = tracklist
            .into_iter()
            .find(|ti| ti.track_number == featured_track_number)
        else {
            return Ok(None);
        };

        let TrackInfo {
            artist,
            audio_track_duration,
            track_number,
            track_title,
            audio_url,
            album_id,
            track_id,
        } = ti;

        let (track_artist_name, track_title) = if artist == "Various Artists" {
            track_title
                .split_once(" - ")
                .or_else(|| track_title.split_once(" : "))
                .or_else(|| track_title.split_once(" – "))
                .map(|(artist, track)| (artist.to_string(), track.to_string()))
                .unwrap_or((artist, track_title))
        } else {
            (artist, track_title)
        };

        let credited_artist = if track_artist_name == band_name {
            None
        } else {
            Some(track_artist_name)
        };

        let release_artist = active_model::Artist {
            id: Set(band_id.to_string()),
            url: Set(band_url.to_string()),
            name: Set(band_name),
            ..Default::default()
        };

        let release_type = ReleaseType::try_from_char(parent_tralbum_type)?;
        match (release_type, album_id) {
            (_, Some(album_id)) if parent_tralbum_id != album_id => {
                anyhow::bail!(
                    "parent_tralbum_id ({parent_tralbum_id}) doesn't match track album_id ({album_id})"
                );
            }
            (ReleaseType::Album, None) => {
                anyhow::bail!(
                    "release is an album (parent_tralbum_id: {parent_tralbum_id}), but the track has no album_id set"
                );
            }
            (ReleaseType::Single, Some(album_id)) => {
                anyhow::bail!(
                    "track is part of a single release but has an album_id ({album_id}) set anyways"
                );
            }
            (ReleaseType::Single, None) if parent_tralbum_id != track_id => {
                anyhow::bail!(
                    "single release, but parent_tralbum_id ({parent_tralbum_id}) doesn't match track track_id ({track_id})"
                );
            }
            _ => {}
        }

        let release = active_model::Release {
            id: Set(parent_tralbum_id.to_string()),
            release_type: Set(release_type),
            url: Set(tralbum_url),
            title: Set(title),
            artist_id: Set(band_id.to_string()),
            ..Default::default()
        };

        let download_url = audio_url.into_iter().next_back().map(|(_k, v)| v);

        let track = active_model::Track {
            id: Set(track_id.to_string()),
            title: Set(track_title),
            duration: Set(audio_track_duration),
            download_url: download_url.map(|url| Set(Some(url))).unwrap_or(NotSet),
            credited_artist: Set(credited_artist),
            release_id: Set(parent_tralbum_id.to_string()),
            release_track_number: Set(track_number as u16),
            ..Default::default()
        };

        Ok(Some((track, release, release_artist)))
    }
}

#[derive(Debug)]
pub(crate) struct PostModels {
    pub(crate) post: active_model::Post,
    pub(crate) items: Vec<(
        active_model::PostTrack,
        active_model::Track,
        active_model::Release,
        active_model::Artist,
    )>,
}

impl active_model::Scrape {
    fn try_from_html<T: Into<Vec<u8>>>(url: &str, bytes: T) -> anyhow::Result<Self> {
        let html = String::from_utf8(bytes.into())?;

        let doc = Html::parse_document(&html);

        let mut metadata = std::collections::HashMap::new();
        for elem in POST_META.select(&doc) {
            let Some(prop) = elem.attr("property") else {
                continue;
            };
            let Some(value) = elem.attr("content") else {
                continue;
            };

            let _ = metadata.insert(prop.to_string(), value.to_string());
        }

        let player_data = {
            let Some(player_data) = DAILY_ARTICLE
                .select(&doc)
                .find_map(|elem| elem.attr("data-player-infos"))
            else {
                anyhow::bail!("No data-player-infos track info found in post HTML");
            };

            // sanity check
            let json: Value = serde_json::from_str(player_data)?;
            if !json.is_array() {
                anyhow::bail!("'data-player-infos' was not a JSON array");
            }

            compress::compress(player_data)?
        };

        let now = chrono::Utc::now();

        Ok(active_model::Scrape {
            url: Set(url.to_string()),
            metadata: Set(metadata.into()),
            player_data: Set(player_data),
            created_at: Set(now),
            updated_at: Set(now),
        })
    }
}

impl model::Scrape {
    pub(crate) fn try_as_post_models(&self) -> anyhow::Result<PostModels> {
        let model::Scrape {
            url,
            metadata,
            player_data,
            ..
        } = self;

        let items = {
            let track_json: Vec<Value> = compress::decompress_json::<Vec<Value>>(player_data)?;

            let mut items = Vec::with_capacity(track_json.len());

            let mut post_track_number = 0;
            for value in track_json.into_iter() {
                // nulls can occur in player data, but we currently don't increment track number for
                // them
                if value.is_null() {
                    continue;
                }

                let player_data = PlayerData::deserialize(value)?;
                let featured_track_number = player_data.featured_track_number;

                let Some((track, release, artist)) = player_data.try_into_models()? else {
                    tracing::warn!(
                        "featured track {featured_track_number} for post track {} not found, skipping it",
                        post_track_number + 1
                    );
                    continue;
                };

                post_track_number += 1;

                // FIXME: `track` is an `ActiveModel` because not all fields are
                // set/known at this point, but we _do_ expect `track.id` to be
                // set
                let Some(track_id) = track.id.try_as_ref().map(|id| id.to_owned()) else {
                    anyhow::bail!("post {url} track #{post_track_number} track id is unset");
                };

                let post_track = active_model::PostTrack {
                    post_url: Set(url.clone()),
                    post_track_number: Set(post_track_number as u32),
                    track_id: Set(track_id),
                    ..Default::default()
                };

                items.push((post_track, track, release, artist));
            }

            items
        };

        let post = {
            let mut metadata = metadata.clone();
            let mut get_meta = |name: &str| -> anyhow::Result<String> {
                metadata
                    .remove(name)
                    .ok_or_else(|| anyhow::anyhow!("post metadata value for {name} not found"))
            };

            let published: DateTime = get_meta("article:published_time")?.parse()?;
            let dir = model::Post::derive_post_dir(&published, url);

            active_model::Post {
                url: Set(url.to_owned()),
                title: Set(get_meta("og:title")?),
                description: Set(get_meta("og:description")?),
                published_at: Set(published),
                modified_at: Set(get_meta("article:modified_time")?.parse()?),
                created_at: Set(chrono::Utc::now()),
                updated_at: Set(chrono::Utc::now()),
                dir: Set(dir),
            }
        };

        Ok(PostModels { post, items })
    }
}

pub(crate) async fn urls_from_rss(client: &http::Client) -> anyhow::Result<Vec<String>> {
    let content = client
        .execute(client.get(FEED_URL).build()?)
        .await?
        .bytes()
        .await?;

    Ok(Channel::read_from(&content[..])?
        .into_items()
        .drain(..)
        .map(|item| item.link)
        .filter(Option::is_some)
        .flatten()
        .collect())
}

pub(crate) fn post_url(url: &str) -> Cow<'_, str> {
    if url.starts_with(DAILY_BASE_URL) {
        return Cow::Borrowed(url);
    }

    let path = url.strip_prefix('/').unwrap_or(url);
    Cow::Owned(format!("{DAILY_BASE_URL}/{path}"))
}

pub(crate) fn post_path(url: &str) -> &str {
    url.strip_prefix(DAILY_BASE_URL).unwrap_or(url)
}

pub(crate) async fn scrape(
    url: &str,
    client: &http::Client,
) -> anyhow::Result<active_model::Scrape> {
    tracing::info!("scraping post: {url}");
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let req = client.get(url).build()?;
    let bytes = client
        .execute(req)
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let scrape = active_model::Scrape::try_from_html(url, bytes)?;

    Ok(scrape)
}
