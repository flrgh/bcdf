use crate::types::{BlogPost, DateTime, Duration, Track};
use anyhow::Context;
use scraper::{Html, Selector};
use serde_json as json;
use std::sync::LazyLock;

pub(crate) const FEED_URL: &str = "https://daily.bandcamp.com/feed/";

struct HtmlSelector {
    text: String,
    selector: Selector,
}

impl HtmlSelector {
    fn try_new(s: &str) -> anyhow::Result<Self> {
        match Selector::parse(s) {
            Ok(sel) => Ok(Self {
                text: s.to_string(),
                selector: sel,
            }),
            Err(e) => {
                anyhow::bail!("failed parsing CSS selector from '{s}': {e}");
            }
        }
    }

    fn try_new_meta(name: &str) -> anyhow::Result<Self> {
        let s = format!(r#"meta[property="{name}"]"#);
        Self::try_new(s.as_str())
    }

    fn select<'a, 'b>(&'a self, doc: &'b Html) -> scraper::html::Select<'b, 'a> {
        doc.select(&self.selector)
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
selector!(META_TITLE, "og:title", HtmlSelector::try_new_meta);
selector!(META_URL, "og:url", HtmlSelector::try_new_meta);
selector!(
    META_PUBLISHED,
    "article:published_time",
    HtmlSelector::try_new_meta
);
selector!(
    META_MODIFIED,
    "article:modified_time",
    HtmlSelector::try_new_meta
);
selector!(
    META_DESCRIPTION,
    "og:description",
    HtmlSelector::try_new_meta
);

#[derive(Debug, PartialEq, Clone, serde::Deserialize)]
pub(crate) struct TrackInfo {
    pub(crate) artist: String,

    #[serde(deserialize_with = "crate::util::duration_from_f64")]
    pub(crate) audio_track_duration: Duration,
    pub(crate) track_number: usize,
    pub(crate) track_title: String,
    pub(crate) audio_url: std::collections::BTreeMap<String, String>,

    pub(crate) album_id: Option<u64>,
    pub(crate) track_id: u64,
}

impl TrackInfo {
    fn download_url(&self) -> Option<String> {
        self.audio_url.last_key_value().map(|(_, v)| v.to_owned())
    }
}

#[derive(Debug, PartialEq, serde::Deserialize)]
pub(crate) struct PlayerData {
    pub(crate) title: String,
    pub(crate) tracklist: Vec<TrackInfo>,
    pub(crate) featured_track_number: usize,

    pub(crate) band_name: String,
    pub(crate) band_id: Option<u64>,
    pub(crate) band_location: Option<String>,
    pub(crate) band_url: Option<String>,
    pub(crate) tralbum_url: Option<String>,
}

impl PlayerData {
    pub(crate) fn get_track(&self, playlist_index: usize) -> Option<Track> {
        self.tracklist
            .iter()
            .find(|&ti| ti.track_number == self.featured_track_number)
            .map(|ti| Track {
                title: ti.track_title.clone(),
                artist: crate::types::Artist {
                    name: ti.artist.clone(),
                    bandcamp_id: if ti.artist == self.band_name {
                        self.band_id
                    } else {
                        None
                    },
                    bandcamp_url: if ti.artist == self.band_name {
                        self.band_url.clone()
                    } else {
                        None
                    },
                    spotify_id: None,
                },
                album_artist: crate::types::Artist {
                    name: self.band_name.clone(),
                    bandcamp_id: self.band_id,
                    bandcamp_url: self.band_url.clone(),
                    spotify_id: None,
                },
                album: crate::types::Album {
                    title: self.title.clone(),
                    bandcamp_id: ti.album_id,
                    bandcamp_url: self.tralbum_url.clone(),
                    spotify_id: None,
                },
                duration: ti.audio_track_duration,
                album_track_number: ti.track_number,
                download_url: ti.download_url(),
                bandcamp_id: ti.track_id,
                spotify_id: None,
                spotify_match_score: None,
                spotify_playlist_id: None,
                post_track_number: playlist_index,
                filename: None,
            })
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct BlogMeta {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) published: DateTime,
    pub(crate) modified: DateTime,
    pub(crate) description: String,
}

impl BlogMeta {
    pub(crate) fn try_from_html(doc: &Html) -> anyhow::Result<Self> {
        fn get_meta(doc: &Html, selector: &HtmlSelector) -> anyhow::Result<String> {
            // TODO: memoize or lazy_static all of the properties we use
            selector
                .select(doc)
                .find_map(|elem| elem.attr("content"))
                .map(|res| res.to_owned())
                .ok_or_else(|| {
                    anyhow::format_err!("no metadata content found for '{}'", selector.text)
                })
        }

        Ok(Self {
            title: get_meta(doc, &META_TITLE)?,
            url: get_meta(doc, &META_URL)?,
            published: get_meta(doc, &META_PUBLISHED)?.parse()?,
            modified: get_meta(doc, &META_MODIFIED)?.parse()?,
            description: get_meta(doc, &META_DESCRIPTION)?,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Scrape {
    pub(crate) html: String,
    pub(crate) fetched_at: DateTime,
}

impl Scrape {
    pub(crate) async fn fetch(url: &str, client: &reqwest::Client) -> anyhow::Result<Self> {
        let req = client.get(url).build()?;
        let bytes = client
            .execute(req)
            .await?
            .error_for_status()?
            .bytes()
            .await?;

        Ok(Self {
            html: String::from_utf8(bytes.to_vec())?,
            fetched_at: chrono::Utc::now(),
        })
    }

    pub(crate) fn parse(&self) -> anyhow::Result<BlogPost> {
        let doc = Html::parse_document(&self.html);

        let meta = BlogMeta::try_from_html(&doc).context("extracting blog metadata from HTML")?;

        let tracks = {
            let mut tracks = Vec::new();
            let mut idx = 0;
            for matched in DAILY_ARTICLE
                .select(&doc)
                .filter_map(|elem| elem.attr("data-player-infos"))
            {
                let infos: Vec<json::Value> =
                    json::from_str(matched).context("parsing 'data-player-infos' as JSON")?;

                for info in infos.into_iter().filter(|v| !v.is_null()) {
                    let info: PlayerData = json::from_value(info).context(format!(
                        "parsing {} from JSON",
                        std::any::type_name::<PlayerData>()
                    ))?;
                    idx += 1;
                    if let Some(track) = info.get_track(idx) {
                        tracks.push(track);
                    }
                }
            }
            tracks
        };

        let BlogMeta {
            title,
            url,
            published,
            modified,
            description,
        } = meta;

        Ok(BlogPost {
            dir: BlogPost::derive_post_dir(&published, &title),
            url,
            title,
            description,
            published,
            modified,
            tracks,
            spotify_playlist: None,
        })
    }
}

pub(crate) async fn scrape(url: &str, client: &reqwest::Client) -> anyhow::Result<Scrape> {
    Scrape::fetch(url, client).await
}
