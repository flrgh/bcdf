use std::collections::BTreeMap;
use std::sync::Mutex;
use strum::IntoEnumIterator;

#[derive(
    Debug,
    PartialOrd,
    Ord,
    Clone,
    Hash,
    Eq,
    PartialEq,
    strum::Display,
    strum::EnumCount,
    strum::EnumIter,
)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum Metric {
    BlogPostsSeen,
    SpotifyPlaylistsCreated,
    TracksDownloaded,
    TracksDiscoveredOnSpotify,
    TracksMissingFromSpotify,
    TracksAddedToSpotifyPlaylist,
    TracksWithUpdatedTags,
    TracksSeen,
    SpotifyTrackSearchQueries,
    SpotifyErrors,
}

type Metrics = BTreeMap<Metric, usize>;

lazy_static! {
    static ref METRICS: Mutex<Metrics> = {
        let mut map = BTreeMap::new();
        for m in Metric::iter() {
            map.insert(m, 0);
        }

        Mutex::new(map)
    };
    pub(crate) static ref MAX_STRING_WIDTH: usize = {
        Metric::iter()
            .map(|m| m.to_string().len())
            .max()
            .unwrap_or(0)
    };
}

fn metrics<'a>() -> std::sync::MutexGuard<'a, Metrics> {
    METRICS.lock().expect("metrics lock is poisoned!")
}

pub(crate) fn inc(metric: Metric, n: usize) {
    let mut metrics = metrics();
    *metrics.entry(metric).or_insert(0) += n;
}

pub(crate) fn summarize() -> Metrics {
    metrics().clone()
}

pub(crate) use Metric::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_name() {
        assert_eq!("blog_posts_seen", BlogPostsSeen.to_string());
    }

    #[test]
    fn all_metrics_in_static_metrics() {
        let metrics = metrics();
        for m in Metric::iter() {
            assert!(metrics.contains_key(&m));
        }
    }

    #[test]
    fn all_metrics_in_summary() {
        let summary = summarize();
        for m in Metric::iter() {
            assert!(summary.contains_key(&m));
        }
    }
}
