use crate::types::{DateTime, Duration, Track};
use scraper::{Html, Selector};
use serde_json as json;

pub(crate) const FEED_URL: &str = "https://daily.bandcamp.com/feed/";

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct TrackInfo {
    pub(crate) artist: String,

    #[serde(deserialize_with = "crate::util::duration_from_f64")]
    pub(crate) audio_track_duration: Duration,
    pub(crate) track_number: usize,
    pub(crate) track_title: String,
    pub(crate) audio_url: std::collections::BTreeMap<String, String>,
}

impl TrackInfo {
    fn download_url(&self) -> Option<String> {
        self.audio_url.last_key_value().map(|(_, v)| v.to_owned())
    }
}

impl Track {
    fn from(ti: &TrackInfo, album: &str) -> Self {
        Self {
            title: ti.track_title.to_owned(),
            artist: ti.artist.to_owned(),
            album: album.to_owned(),
            duration: ti.audio_track_duration,
            number: ti.track_number,
            download_url: ti.download_url(),
        }
    }
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PlayerInfo {
    pub(crate) title: String,
    pub(crate) tracklist: Vec<TrackInfo>,
    pub(crate) featured_track_number: usize,
}

impl PlayerInfo {
    pub(crate) fn get_track(&self) -> Option<Track> {
        self.tracklist
            .iter()
            .find(|&ti| ti.track_number == self.featured_track_number)
            .map(|ti| Track::from(ti, &self.title))
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

fn get_meta(doc: &Html, name: &str) -> Option<String> {
    let title = Selector::parse(&format!(r#"meta[property="{name}"]"#)).unwrap();

    doc.select(&title)
        .map(|elem| elem.attr("content").unwrap().to_owned())
        .next()
}

impl BlogInfo {
    pub(crate) fn from_html(html: &str) -> Self {
        let doc = Html::parse_document(html);

        let article = Selector::parse("#p-daily-article").unwrap();

        let mut tracks = vec![];
        let mut raw: Vec<json::Value> = vec![];

        for elem in doc.select(&article) {
            if let Some(infos) = elem.attr("data-player-infos") {
                raw.push(json::from_str(infos).unwrap());

                let parsed: Vec<PlayerInfo> = json::from_str(infos).unwrap();
                for info in parsed.iter() {
                    if let Some(track) = info.get_track() {
                        tracks.push(track.clone());
                    }
                }
            }
        }

        let title = get_meta(&doc, "og:title").unwrap();
        let published = get_meta(&doc, "article:published_time").unwrap();
        let modified = get_meta(&doc, "article:modified_time").unwrap();
        let url = get_meta(&doc, "og:url").unwrap();
        let description = get_meta(&doc, "og:description").unwrap();

        Self {
            published: published.parse().unwrap(),
            modified: modified.parse().unwrap(),
            title,
            url,
            description,
            tracks,
            raw,
        }
    }
}
