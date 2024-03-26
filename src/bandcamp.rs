use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json as json;

use crate::types::Track;

pub(crate) const FEED_URL: &str = "https://daily.bandcamp.com/feed/";

#[derive(Debug, PartialEq, Deserialize, Clone)]
pub(crate) struct TrackInfo {
    pub(crate) artist: String,
    pub(crate) audio_track_duration: f64,
    pub(crate) track_number: usize,
    pub(crate) track_title: String,
}

#[derive(Debug, PartialEq, Deserialize)]
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
            .map(|ti| Track {
                title: ti.track_title.to_owned(),
                artist: ti.artist.to_owned(),
                album: self.title.to_owned(),
                duration: chrono::Duration::try_seconds(ti.audio_track_duration as i64).unwrap(),
                number: ti.track_number,
            })
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct BlogInfo {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) published: String,
    pub(crate) modified: String,
    pub(crate) description: String,
    pub(crate) tracks: Vec<Track>,
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

        for elem in doc.select(&article) {
            if let Some(infos) = elem.attr("data-player-infos") {
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
            published: published.to_owned(),
            modified: modified.to_owned(),
            title,
            url,
            description,
            tracks,
        }
    }
}
