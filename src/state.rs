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
            Ok(fh) => {
                let mut state = match json::from_reader::<_, State>(fh) {
                    Ok(mut state) => {
                        state.blog_info = info;
                        state
                    }
                    Err(_) => State::new(info)
                };
                state.fname = path;
                state.save()?;
                state
            },
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
        if self.fname.parent().is_none() {
            eprintln!("what the fuck: {}", self.fname.to_string_lossy());
        }

        self.fname.parent().unwrap().to_path_buf()
    }

    pub(crate) fn save(&self) -> anyhow::Result<()> {
        let mut fh = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&self.fname)?;

        json::to_writer(&mut fh, &self)?;
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct TrackState {}
