use crate::bandcamp::BlogInfo;
use serde_json as json;
use std::path::PathBuf;

const OUT_DIR: &str = "./data";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct State {
    pub(crate) blog_info: BlogInfo,
    pub(crate) tracks: Option<Vec<TrackState>>,

    #[serde(skip)]
    pub(crate) fname: PathBuf,
}

impl State {
    fn new(info: BlogInfo) -> Self {
        let fname = Self::filename(&info);
        Self {
            blog_info: info,
            tracks: None,
            fname,
        }
    }

    fn filename(info: &BlogInfo) -> PathBuf {
        PathBuf::from(OUT_DIR)
            .join(format!(
                "{} - {}",
                info.published.format("%Y-%m-%d"),
                info.title
            ))
            .join("info.json")
    }

    pub(crate) fn try_get_or_create(info: BlogInfo) -> anyhow::Result<Self> {
        let path = Self::filename(&info);

        let state: State = match std::fs::File::open(&path) {
            Ok(fh) => json::from_reader(fh)?,
            Err(_) => {
                std::fs::create_dir_all(path.parent().unwrap())?;
                let fh = std::fs::File::create(&path)?;
                let state = Self::new(info);
                json::to_writer(fh, &state)?;
                state
            }
        };

        Ok(state)
    }

    pub(crate) fn dirname(&self) -> PathBuf {
        self.fname.parent().unwrap().to_path_buf()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct TrackState {}
