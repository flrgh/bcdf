use crate::list::ColumnSpec::{Num, Pin, Plain};
use crate::list::{self, Filters};
use crate::query::{normalize, Query};
use crate::store::Store;
use crate::track::{self, TrackRow};
use crate::types::{BlogPost, BlogPostRow, Format, SpotifyPlaylist, Track};
use chrono::SecondsFormat;
use clap::Subcommand;
use comfy_table::Cell;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::io::Write;

/// View and manage Spotify playlists
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

#[derive(Subcommand, Debug)]
enum Command {
    /// List playlists
    Ls(List),

    /// Show a single playlist
    Show(Show),
}

#[derive(clap::Args, Debug)]
pub(crate) struct List {
    /// Select playlists matching this text
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
        let posts = store.list_posts()?;

        let mut rows: Vec<(&BlogPostRow, &SpotifyPlaylist)> = posts
            .iter()
            .filter(|row| self.filters.published_at(&row.post.published))
            .filter(|row| match &self.query {
                Some(query) => query.matches(*row),
                None => true,
            })
            .filter_map(|row| {
                row.post
                    .spotify_playlist
                    .as_ref()
                    .map(|playlist| (row, playlist))
            })
            .collect();

        rows.sort_by(|a, b| {
            if self.reverse {
                self.sort.compare(a, b).reverse()
            } else {
                self.sort.compare(a, b)
            }
        });

        self.filters.limit_results(&mut rows);

        let mut out = std::io::stdout().lock();

        match self.output {
            Format::Table => {
                let mut table = list::table([
                    ("Id", Pin),
                    ("Name", Plain),
                    ("Tracks", Num),
                    ("Post", Plain),
                ]);

                for (row, playlist) in rows {
                    table.add_row(vec![
                        Cell::new(list::short_id(&playlist.id)),
                        Cell::new(&playlist.name),
                        Cell::new(row.post.playlist_tracks().count()),
                        Cell::new(&row.locator),
                    ]);
                }

                writeln!(out, "{table}")?;
            }
            Format::Json => {
                let values: Vec<Value> = rows
                    .iter()
                    .map(|(row, playlist)| {
                        json!({
                            "id": playlist.id,
                            "name": playlist.name,
                            "tracks": row.post.playlist_tracks().count(),
                            "post": row.json_summary(),
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
    /// Select the playlist matching this text
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

        let Some(playlist) = &post.spotify_playlist else {
            anyhow::bail!("{} has no spotify playlist", row.locator);
        };

        let mut out = std::io::stdout().lock();
        match self.output {
            Format::Table => {
                let published = post.published.to_rfc3339_opts(SecondsFormat::Secs, true);
                let tracks = post.playlist_tracks().count().to_string();

                let mut table = list::detail_table();
                table.add_rows([
                    ["id", list::short_id(&playlist.id)],
                    ["name", playlist.name.as_str()],
                    ["post", row.locator.as_str()],
                    ["published", published.as_str()],
                    ["tracks", tracks.as_str()],
                ]);

                writeln!(out, "{table}")?;
                writeln!(out, "{}", track::child_table(row, post.playlist_tracks()))?;
            }
            Format::Json => {
                let tracks: Vec<Value> = post
                    .playlist_tracks()
                    .map(|track| TrackRow::new(row, track).child_json())
                    .collect();

                serde_json::to_writer_pretty(
                    &mut out,
                    &json!({
                        "id": playlist.id,
                        "name": playlist.name,
                        "post": row.json_summary(),
                        "tracks": tracks,
                    }),
                )?;

                writeln!(out)?;
            }
        };

        Ok(())
    }
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum Sort {
    /// Sort by newest post
    Published,
    /// Sort by playlist name, case-insensitive
    Name,
    /// Sort by track count, most first
    Tracks,
}

impl Sort {
    fn compare(
        self,
        a: &(&BlogPostRow, &SpotifyPlaylist),
        b: &(&BlogPostRow, &SpotifyPlaylist),
    ) -> Ordering {
        let by_field = match self {
            Self::Published => b.0.post.published.cmp(&a.0.post.published),
            Self::Name => normalize(&a.1.name).cmp(&normalize(&b.1.name)),
            Self::Tracks => {
                b.0.post
                    .playlist_tracks()
                    .count()
                    .cmp(&a.0.post.playlist_tracks().count())
            }
        };

        by_field.then_with(|| a.0.locator.cmp(&b.0.locator))
    }
}

impl BlogPost {
    fn playlist_tracks(&self) -> impl Iterator<Item = &Track> {
        let id = self
            .spotify_playlist
            .as_ref()
            .map(|playlist| playlist.id.as_str());

        self.tracks.iter().filter(
            move |track| match (id, track.spotify_playlist_id.as_deref()) {
                (Some(id), Some(assigned)) => id == assigned,
                _ => false,
            },
        )
    }
}
