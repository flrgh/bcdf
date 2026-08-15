use crate::list::ColumnSpec::{Num, Pin, Plain};
use crate::list::{self, Filters};
use crate::query::{normalize, Matchable, Query};
use crate::store::Store;
use crate::track;
use crate::types::{BlogPostRow, Format};
use chrono::SecondsFormat;
use clap::Subcommand;
use comfy_table::Cell;
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
            .filter(|row| self.filters.published_at(&row.post.published))
            .filter(|row| match &self.query {
                Some(query) => query.matches(row),
                None => true,
            });

        let mut posts: Vec<BlogPostRow> = posts.collect();

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
                let mut table = list::table([
                    ("Post", Pin),
                    ("Tracks", Num),
                    ("Dl", Num),
                    ("Sp", Num),
                    ("Playlist", Pin),
                    ("Title", Plain),
                ]);

                for row in posts.iter() {
                    let post = &row.post;
                    table.add_row(vec![
                        Cell::new(&row.locator),
                        Cell::new(post.tracks.len()),
                        Cell::new(post.downloaded_count()),
                        Cell::new(post.spotify_count()),
                        Cell::new(
                            post.spotify_playlist
                                .as_ref()
                                .map(|p| list::short_id(&p.id))
                                .unwrap_or_default(),
                        ),
                        Cell::new(&post.title),
                    ]);
                }

                writeln!(out, "{table}")?;
            }
            Format::Json => {
                let values: Vec<Value> = posts
                    .iter()
                    .map(|row| {
                        let post = &row.post;
                        json!({
                            "locator": row.locator,
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
        let row = self.query.filter_unique(&posts)?;
        let post = &row.post;

        let mut out = std::io::stdout().lock();
        match self.output {
            Format::Table => {
                let published = post.published.to_rfc3339_opts(SecondsFormat::Secs, true);
                let modified = post.modified.to_rfc3339_opts(SecondsFormat::Secs, true);

                let tracks = format!(
                    "{} ({} downloaded, {} on spotify)",
                    post.tracks.len(),
                    post.downloaded_count(),
                    post.spotify_count()
                );

                let playlist = match &post.spotify_playlist {
                    Some(playlist) => format!(
                        "{} \u{2014} {}",
                        list::short_id(&playlist.id),
                        playlist.name
                    ),
                    None => String::new(),
                };

                let mut table = list::detail_table();
                table.add_rows([
                    ["locator", row.locator.as_str()],
                    ["url", post.url.as_str()],
                    ["title", post.title.as_str()],
                    ["published", published.as_str()],
                    ["modified", modified.as_str()],
                    ["dir", post.dir.as_str()],
                    ["tracks", tracks.as_str()],
                    ["playlist", playlist.as_str()],
                    ["description", post.description.as_str()],
                ]);

                writeln!(out, "{table}")?;
                writeln!(out, "{}", track::child_table(row, &post.tracks))?;
            }
            Format::Json => {
                let tracks: Vec<Value> = row.track_rows().map(|row| row.child_json()).collect();

                serde_json::to_writer_pretty(&mut out, &{
                    json!({
                        "locator": row.locator,
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
    fn compare(self, a: &BlogPostRow, b: &BlogPostRow) -> Ordering {
        let by_field = match self {
            Self::Published => b.post.published.cmp(&a.post.published),
            Self::Title => normalize(&a.post.title).cmp(&normalize(&b.post.title)),
            Self::Dir => a.post.dir.cmp(&b.post.dir),
        };

        by_field.then_with(|| a.locator.cmp(&b.locator))
    }
}

impl Matchable for BlogPostRow {
    fn search_fields(&self) -> Vec<Cow<'_, str>> {
        let post = &self.post;

        let mut fields = vec![
            Cow::from(self.locator.as_str()),
            Cow::from(post.dir.as_str()),
            Cow::from(post.url.as_str()),
            Cow::from(post.title.as_str()),
        ];

        if let Some(playlist) = &post.spotify_playlist {
            fields.push(Cow::from(playlist.id.as_str()));
            fields.push(Cow::from(playlist.name.as_str()));
        }

        for track in &post.tracks {
            fields.push(Cow::from(track.title.as_str()));
            fields.push(Cow::from(track.artist.name.as_str()));
            fields.push(Cow::from(track.album_artist.name.as_str()));
            fields.push(Cow::from(track.album.title.as_str()));
        }

        fields
    }

    fn identity_fields(&self) -> Vec<Cow<'_, str>> {
        let mut fields = vec![
            Cow::from(self.locator.as_str()),
            Cow::from(self.post.url.as_str()),
            Cow::from(self.post.dir.as_str()),
        ];

        // a playlist is 1:1 with its post, so its id names the post too
        if let Some(playlist) = &self.post.spotify_playlist {
            fields.push(Cow::from(playlist.id.as_str()));
        }

        fields
    }
}

impl BlogPostRow {
    /// A post as it appears nested inside a track or playlist
    pub(crate) fn json_summary(&self) -> Value {
        json!({
            "locator": self.locator,
            "url": self.post.url,
            "dir": self.post.dir,
            "title": self.post.title,
            "published": self.post.published,
        })
    }
}
