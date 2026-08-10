use crate::list::Filters;
use crate::query::{normalize, Matchable, Query, DATE_FORMAT};
use crate::store::Store;
use crate::track::{self, TrackRow};
use crate::types::{BlogPost, Format};
use chrono::SecondsFormat;
use clap::Subcommand;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::io::Write;

/// View/manage bandcamp blog posts
#[derive(clap::Args, Debug)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub(crate) fn exec(self, store: &Store) -> anyhow::Result<()> {
        match self.command {
            Command::Ls(ls) => ls.exec(store),
            Command::Show(show) => show.exec(store),
        }
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct List {
    /// Select posts matching this text
    #[arg(value_name = "QUERY")]
    query: Option<Query>,

    #[arg(long, value_enum, default_value = "published")]
    sort: Sort,

    /// Reverse the sort order
    #[arg(short, long)]
    reverse: bool,

    #[command(flatten)]
    filters: Filters,

    #[arg(short, long, value_enum, default_value = "table")]
    output: Format,
}

impl List {
    fn exec(self, store: &Store) -> anyhow::Result<()> {
        let posts = store
            .list_posts()?
            .into_iter()
            .filter(|post| self.filters.published_at(&post.published))
            .filter(|post| match &self.query {
                Some(query) => query.matches(post),
                None => true,
            });

        let mut posts: Vec<BlogPost> = posts.collect();

        posts.sort_by(|a, b| {
            if self.reverse {
                self.sort.compare(a, b).reverse()
            } else {
                self.sort.compare(a, b)
            }
        });

        self.filters.limit_results(&mut posts);

        let mut out = std::io::stdout().lock();

        match self.output {
            Format::Table => {
                writeln!(out, "DIR\tTRACKS\tDOWNLOADED\tSPOTIFY\tPLAYLIST\tTITLE")?;
                for post in posts.into_iter() {
                    write!(out, "{}\t", post.dir)?;
                    write!(out, "{}\t", post.tracks.len())?;
                    write!(out, "{}\t", post.downloaded_count())?;
                    write!(out, "{}\t", post.spotify_count())?;
                    write!(
                        out,
                        "{}\t",
                        post.spotify_playlist
                            .as_ref()
                            .map(|p| p.id.clone())
                            .unwrap_or_default(),
                    )?;
                    write!(out, "{}", post.title)?;

                    writeln!(out)?;
                }
            }
            Format::Json => {
                let values: Vec<Value> = posts
                    .iter()
                    .map(|post| {
                        json!({
                            "dir": post.dir,
                            "url": post.url,
                            "title": post.title,
                            "published": post.published,
                            "modified": post.modified,
                            "tracks": post.tracks.len(),
                            "downloaded": post.downloaded_count(),
                            "spotify": post.spotify_count(),
                            "playlist": post.spotify_playlist,
                        })
                    })
                    .collect();

                serde_json::to_writer_pretty(&mut out, &values)?;
                writeln!(out)?;
            }
        };

        Ok(())
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct Show {
    /// Select the post matching this text
    #[arg(value_name = "QUERY")]
    query: Query,

    #[arg(short, long, value_enum, default_value = "table")]
    output: Format,
}

impl Show {
    fn exec(self, store: &Store) -> anyhow::Result<()> {
        let posts = store.list_posts()?;
        let post = self.query.filter_unique(&posts)?;

        let mut out = std::io::stdout().lock();
        match self.output {
            Format::Table => {
                let published = post.published.to_rfc3339_opts(SecondsFormat::Secs, true);
                let modified = post.modified.to_rfc3339_opts(SecondsFormat::Secs, true);

                writeln!(out, "url\t{}", post.url)?;
                writeln!(out, "title\t{}", post.title)?;
                writeln!(out, "published\t{published}")?;
                writeln!(out, "modified\t{modified}")?;
                writeln!(out, "dir\t{}", post.dir)?;
                writeln!(
                    out,
                    "tracks\t{} ({} downloaded, {} on spotify)",
                    post.tracks.len(),
                    post.downloaded_count(),
                    post.spotify_count()
                )?;

                match &post.spotify_playlist {
                    Some(playlist) => {
                        writeln!(out, "playlist\t{} \u{2014} {}", playlist.id, playlist.name)?
                    }
                    None => writeln!(out, "playlist\t")?,
                }

                writeln!(out, "description\t{}", post.description)?;
                writeln!(out)?;

                writeln!(out, "{}", track::CHILD_HEADER)?;
                for track in &post.tracks {
                    TrackRow::new(post, track).write_child_row(&mut out)?;
                }
            }
            Format::Json => {
                let tracks: Vec<Value> = post
                    .tracks
                    .iter()
                    .map(|track| TrackRow::new(post, track).child_json())
                    .collect();

                serde_json::to_writer_pretty(&mut out, &{
                    json!({
                        "dir": post.dir,
                        "url": post.url,
                        "title": post.title,
                        "description": post.description,
                        "published": post.published,
                        "modified": post.modified,
                        "playlist": post.spotify_playlist,
                        "tracks": tracks,
                    })
                })?;

                writeln!(out)?;
            }
        };
        Ok(())
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List posts
    Ls(List),

    /// Show a single post
    Show(Show),
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum Sort {
    /// Sort by date published (newest first)
    Published,
    /// Sort by title, case-insensitive
    Title,
    /// Sort by post directory, case-insensitive
    Dir,
}

impl Sort {
    fn compare(self, a: &BlogPost, b: &BlogPost) -> Ordering {
        let by_field = match self {
            Self::Published => b.published.cmp(&a.published),
            Self::Title => normalize(&a.title).cmp(&normalize(&b.title)),
            Self::Dir => a.dir.cmp(&b.dir),
        };

        by_field.then_with(|| a.dir.cmp(&b.dir))
    }
}

impl Matchable for BlogPost {
    fn search_fields(&self) -> Vec<Cow<'_, str>> {
        let mut fields = vec![
            Cow::from(self.dir.as_str()),
            Cow::from(self.url.as_str()),
            Cow::from(self.title.as_str()),
            Cow::from(self.published.format(DATE_FORMAT).to_string()),
        ];

        if let Some(playlist) = &self.spotify_playlist {
            fields.push(Cow::from(playlist.id.as_str()));
            fields.push(Cow::from(playlist.name.as_str()));
        }

        for track in &self.tracks {
            fields.push(Cow::from(track.title.as_str()));
            fields.push(Cow::from(track.artist.name.as_str()));
            fields.push(Cow::from(track.album_artist.name.as_str()));
            fields.push(Cow::from(track.album.title.as_str()));
        }

        fields
    }

    fn identity_fields(&self) -> Vec<Cow<'_, str>> {
        let mut fields = vec![Cow::from(self.url.as_str()), Cow::from(self.dir.as_str())];

        // a playlist is 1:1 with its post, so its id names the post too
        if let Some(playlist) = &self.spotify_playlist {
            fields.push(Cow::from(playlist.id.as_str()));
        }

        fields
    }
}

impl BlogPost {
    /// A post as it appears nested inside a track or playlist
    pub(crate) fn json_summary(&self) -> Value {
        json!({
            "url": self.url,
            "dir": self.dir,
            "title": self.title,
            "published": self.published,
        })
    }
}
