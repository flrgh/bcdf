use crate::types::{DateTime, Duration, Track};
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

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct TrackInfo {
    pub(crate) artist: String,

    #[serde(deserialize_with = "crate::util::duration_from_f64")]
    pub(crate) audio_track_duration: Duration,
    pub(crate) track_number: usize,
    pub(crate) track_title: String,
    pub(crate) audio_url: std::collections::BTreeMap<String, String>,

    pub(crate) album_id: Option<u64>,
    pub(crate) track_id: Option<u64>,
}

impl TrackInfo {
    fn download_url(&self) -> Option<String> {
        self.audio_url.last_key_value().map(|(_, v)| v.to_owned())
    }
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PlayerInfo {
    pub(crate) title: String,
    pub(crate) tracklist: Vec<TrackInfo>,
    pub(crate) featured_track_number: usize,

    pub(crate) band_name: String,
    pub(crate) band_id: Option<u64>,
    pub(crate) band_location: Option<String>,
    pub(crate) band_url: Option<String>,
    pub(crate) tralbum_url: Option<String>,
}

impl PlayerInfo {
    pub(crate) fn get_track(&self, playlist_index: usize) -> Option<Track> {
        self.tracklist
            .iter()
            .find(|&ti| ti.track_number == self.featured_track_number)
            .map(|ti| Track {
                title: ti.track_title.clone(),
                artist: crate::types::Artist {
                    name: ti.artist.clone(),
                    bandcamp_id: if ti.artist == self.band_name {
                        self.band_id.map(|id| id.to_string())
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
                    bandcamp_id: self.band_id.map(|id| id.to_string()),
                    bandcamp_url: self.band_url.clone(),
                    spotify_id: None,
                },
                album: crate::types::Album {
                    title: self.title.clone(),
                    bandcamp_id: ti.album_id.map(|id| id.to_string()),
                    bandcamp_url: self.tralbum_url.clone(),
                    spotify_id: None,
                },
                duration: ti.audio_track_duration,
                number: ti.track_number,
                download_url: ti.download_url(),
                bandcamp_track_id: ti.track_id.map(|id| id.to_string()),
                spotify_id: None,
                spotify_playlist_id: None,
                bandcamp_playlist_track_number: playlist_index,
            })
    }
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct BlogInfo {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) published: DateTime,
    pub(crate) modified: DateTime,
    pub(crate) description: String,
    pub(crate) tracks: Vec<Track>,
    pub(crate) raw: Vec<json::Value>,
}

fn get_meta(doc: &Html, name: &str) -> anyhow::Result<String> {
    // TODO: memoize or lazy_static all of the properties we use
    let selector = css_selector(&format!(r#"meta[property="{name}"]"#))?;
    doc.select(&selector)
        .find_map(|elem| elem.attr("content"))
        .map(|res| res.to_owned())
        .ok_or_else(|| anyhow::format_err!("no metadata content found for '{name}'"))
}

impl BlogInfo {
    pub(crate) fn from_html(html: &str) -> anyhow::Result<Self> {
        let doc = Html::parse_document(html);

        // TODO: lazy_static
        let article = css_selector("#p-daily-article")?;

        let mut tracks = vec![];
        let mut raw: Vec<json::Value> = vec![];

        let mut idx = 0;

        for elem in doc.select(&article) {
            if let Some(infos) = elem.attr("data-player-infos") {
                raw.push(json::from_str(infos)?);

                let parsed: Vec<PlayerInfo> =
                    json::from_str(infos).context("parsing PlayerInfo from BandCamp blog HTML")?;

                for info in parsed.iter() {
                    idx += 1;
                    if let Some(mut track) = info.get_track(idx) {
                        track.bandcamp_playlist_track_number = idx;
                        tracks.push(track.clone());
                    }
                }
            }
        }

        let title = get_meta(&doc, "og:title")?;
        let published = get_meta(&doc, "article:published_time")?;
        let modified = get_meta(&doc, "article:modified_time")?;
        let url = get_meta(&doc, "og:url")?;
        let description = get_meta(&doc, "og:description")?;
        let published = published.parse()?;
        let modified = modified.parse()?;

        Ok(Self {
            published,
            modified,
            title,
            url,
            description,
            tracks,
            raw,
        })
    }

    pub(crate) async fn try_from_url(url: &str, client: &reqwest::Client) -> anyhow::Result<Self> {
        let req = client.get(url).build()?;
        let bytes = client
            .execute(req)
            .await?
            .error_for_status()?
            .bytes()
            .await?;

        let html = String::from_utf8(bytes.to_vec())?;
        Self::from_html(&html)
    }
}
