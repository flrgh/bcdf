use rss::Channel;
use scraper::{Html, Selector};
use sea_orm::ActiveValue::{NotSet, Set};
use std::{borrow::Cow, collections::HashMap, sync::LazyLock};

use crate::db::{active_model, model, release::Type as ReleaseType};
use crate::types::{DateTime, Duration};

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

fn duration_from_f64<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    let secs: f64 = serde::de::Deserialize::deserialize(deserializer)?;
    Ok(Duration::from_secs_f64(secs))
}

#[derive(Debug, PartialEq, Clone, serde::Deserialize)]
pub(crate) struct TrackInfo {
    pub(crate) artist: String,

    #[serde(deserialize_with = "duration_from_f64")]
    pub(crate) audio_track_duration: Duration,
    pub(crate) track_number: usize,
    pub(crate) track_title: String,
    pub(crate) audio_url: std::collections::BTreeMap<String, String>,

    pub(crate) album_id: Option<u64>,
    pub(crate) track_id: u64,
}

#[derive(Debug, PartialEq, Clone, serde::Deserialize)]
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
    ) -> anyhow::Result<(
        active_model::Track,
        active_model::Release,
        active_model::Artist,
    )> {
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
            anyhow::bail!("featured track #{featured_track_number} not found");
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
            duration: Set(audio_track_duration.as_secs_f64()),
            download_url: download_url.map(|url| Set(Some(url))).unwrap_or(NotSet),
            credited_artist: Set(credited_artist),
            release_id: Set(parent_tralbum_id.to_string()),
            release_track_number: Set(track_number as u16),
            ..Default::default()
        };

        Ok((track, release, release_artist))
    }
}

#[derive(Debug)]
pub(crate) struct ScrapeModels {
    pub(crate) scrape: active_model::Scrape,
    pub(crate) post: active_model::Post,
    pub(crate) items: Vec<(
        active_model::PostTrack,
        active_model::Track,
        active_model::Release,
        active_model::Artist,
    )>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScrapeTask {
    url: String,
    metadata: HashMap<String, String>,
    raw_track_info: String,
    tracks: Vec<Option<PlayerData>>,
}

impl ScrapeTask {
    pub(crate) async fn fetch(url: &str, client: &reqwest::Client) -> anyhow::Result<Self> {
        let req = client.get(url).build()?;
        let bytes = client
            .execute(req)
            .await?
            .error_for_status()?
            .bytes()
            .await?;

        Self::try_from_html(url, bytes)
    }

    fn tracks_from_doc(doc: &Html) -> anyhow::Result<(String, Vec<Option<PlayerData>>)> {
        let Some(raw_track_info) = DAILY_ARTICLE
            .select(doc)
            .find_map(|elem| elem.attr("data-player-infos"))
        else {
            anyhow::bail!("No data-player-infos track info found in post HTML");
        };

        let json: serde_json::Value = serde_json::from_str(raw_track_info)?;
        let serde_json::Value::Array(arr) = json else {
            anyhow::bail!("'data-player-infos' was not a JSON array");
        };

        let track_info = arr
            .into_iter()
            .map(|v| {
                if v.is_null() {
                    Ok(None)
                } else {
                    match serde_json::from_value(v.clone()) {
                        Ok(pd) => Ok(Some(pd)),
                        Err(e) => Err(anyhow::anyhow!("woops! {e}\n{v}\n")),
                    }
                }
            })
            .collect::<Result<Vec<Option<PlayerData>>, anyhow::Error>>()?;

        Ok((raw_track_info.to_string(), track_info))
    }

    pub(crate) fn try_from_html<T: Into<Vec<u8>>>(url: &str, bytes: T) -> anyhow::Result<Self> {
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

        let (raw_track_info, tracks) = Self::tracks_from_doc(&doc)?;

        Ok(Self {
            url: url.to_string(),
            metadata,
            raw_track_info,
            tracks,
        })
    }

    pub(crate) fn try_into_models(self) -> anyhow::Result<ScrapeModels> {
        let ScrapeTask {
            url,
            mut metadata,
            raw_track_info,
            tracks: track_json,
        } = self;

        let scrape = active_model::Scrape {
            url: Set(url.clone()),
            metadata: Set(metadata.clone().into()),
            player_data: Set(crate::compress::compress(&raw_track_info)?),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        };

        let mut items = Vec::with_capacity(track_json.len());

        for (i, player_data) in track_json.into_iter().flatten().enumerate() {
            let post_track_number = i + 1;
            let (track, release, artist) = player_data.try_into_models()?;

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

        let post = {
            let mut get_meta = |name: &str| -> anyhow::Result<String> {
                metadata
                    .remove(name)
                    .ok_or_else(|| anyhow::anyhow!("post metadata value for {name} not found"))
            };

            let published: DateTime = get_meta("article:published_time")?.parse()?;
            let dir = model::Post::derive_post_dir(&published, &url);

            active_model::Post {
                url: Set(url.clone()),
                title: Set(get_meta("og:title")?),
                description: Set(get_meta("og:description")?),
                published_at: Set(published),
                modified_at: Set(get_meta("article:modified_time")?.parse()?),
                created_at: Set(chrono::Utc::now()),
                updated_at: Set(chrono::Utc::now()),
                dir: Set(dir),
            }
        };

        Ok(ScrapeModels {
            scrape,
            post,
            items,
        })
    }
}

// impl model::Scrape {
//     fn try_into_upsert(self) -> anyhow::Result<ScrapeModels> {
//         let model::Scrape {
//             url,
//             mut metadata,
//             player_data,
//             created_at,
//             updated_at,
//         } = self;
//
//         let track_json: Vec<serde_json::Value> = crate::compress::decompress_json(&player_data)?;
//
//         let scrape = active_model::Scrape {
//             url: Set(url.clone()),
//             metadata: Set(metadata.clone().into()),
//             player_data: Set(player_data),
//             created_at: Set(created_at),
//             updated_at: Set(updated_at),
//         };
//
//         let mut items = Vec::with_capacity(track_json.len());
//
//         for (i, player_data) in track_json.into_iter().flatten().enumerate() {
//             let post_track_number = i + 1;
//             let (track, release, artist) = player_data.try_into_models()?;
//
//             // FIXME: `track` is an `ActiveModel` because not all fields are
//             // set/known at this point, but we _do_ expect `track.id` to be
//             // set
//             let Some(track_id) = track.id.try_as_ref().map(|id| id.to_owned()) else {
//                 anyhow::bail!("post {url} track #{post_track_number} track id is unset");
//             };
//
//             let post_track = active_model::PostTrack {
//                 post_url: Set(url.clone()),
//                 post_track_number: Set(post_track_number as u32),
//                 track_id: Set(track_id),
//                 ..Default::default()
//             };
//
//             items.push((post_track, track, release, artist));
//         }
//
//         let post = {
//             let mut get_meta = |name: &str| -> anyhow::Result<String> {
//                 metadata
//                     .remove(name)
//                     .ok_or_else(|| anyhow::anyhow!("post metadata value for {name} not found"))
//             };
//
//             let published: DateTime = get_meta("article:published_time")?.parse()?;
//             let dir = model::Post::derive_post_dir(&published, &url);
//
//             active_model::Post {
//                 url: Set(url.clone()),
//                 title: Set(get_meta("og:title")?),
//                 description: Set(get_meta("og:description")?),
//                 published_at: Set(published),
//                 modified_at: Set(get_meta("article:modified_time")?.parse()?),
//                 created_at: Set(chrono::Utc::now()),
//                 updated_at: Set(chrono::Utc::now()),
//                 dir: Set(dir),
//             }
//         };
//
//         Ok(ScrapeModels {
//             scrape,
//             post,
//             items,
//         })
//
//     }
// }

pub(crate) async fn scrape(url: &str, client: &reqwest::Client) -> anyhow::Result<ScrapeTask> {
    ScrapeTask::fetch(url, client).await
}

pub(crate) async fn urls_from_rss(client: &reqwest::Client) -> anyhow::Result<Vec<String>> {
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
