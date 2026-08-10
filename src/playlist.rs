use crate::list::Filters;
use crate::query::{normalize, Query};
use crate::store::Store;
use crate::track::{self, TrackRow};
use crate::types::{BlogPost, Format, SpotifyPlaylist, Track};
use chrono::SecondsFormat;
use clap::Subcommand;
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

        let mut rows: Vec<(&BlogPost, &SpotifyPlaylist)> = posts
            .iter()
            .filter(|post| self.filters.published_at(&post.published))
            .filter(|post| match &self.query {
                Some(query) => query.matches(*post),
                None => true,
            })
            .filter_map(|post| {
                post.spotify_playlist
                    .as_ref()
                    .map(|playlist| (post, playlist))
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
                writeln!(out, "ID\tTRACKS\tPOST\tNAME")?;
                for (post, playlist) in rows {
                    write!(out, "{}\t", playlist.id)?;
                    write!(out, "{}\t", post.playlist_tracks().count())?;
                    write!(out, "{}\t", post.dir)?;
                    writeln!(out, "{}", playlist.name)?;
                }
            }
            Format::Json => {
                let values: Vec<Value> = rows
                    .iter()
                    .map(|(post, playlist)| {
                        json!({
                            "id": playlist.id,
                            "name": playlist.name,
                            "tracks": post.playlist_tracks().count(),
                            "post": post.json_summary(),
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
        let post = self.query.filter_unique(&posts)?;

        let Some(playlist) = &post.spotify_playlist else {
            anyhow::bail!("{} has no spotify playlist", post.dir);
        };

        let mut out = std::io::stdout().lock();
        match self.output {
            Format::Table => {
                let published = post.published.to_rfc3339_opts(SecondsFormat::Secs, true);

                writeln!(out, "id\t{}", playlist.id)?;
                writeln!(out, "name\t{}", playlist.name)?;
                writeln!(out, "post\t{}", post.dir)?;
                writeln!(out, "published\t{published}")?;
                writeln!(out, "tracks\t{}", post.playlist_tracks().count())?;
                writeln!(out)?;

                writeln!(out, "{}", track::CHILD_HEADER)?;
                for track in post.playlist_tracks() {
                    TrackRow::new(post, track).write_child_row(&mut out)?;
                }
            }
            Format::Json => {
                let tracks: Vec<Value> = post
                    .playlist_tracks()
                    .map(|track| TrackRow::new(post, track).child_json())
                    .collect();

                serde_json::to_writer_pretty(
                    &mut out,
                    &json!({
                        "id": playlist.id,
                        "name": playlist.name,
                        "post": post.json_summary(),
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
        a: &(&BlogPost, &SpotifyPlaylist),
        b: &(&BlogPost, &SpotifyPlaylist),
    ) -> Ordering {
        let by_field = match self {
            Self::Published => b.0.published.cmp(&a.0.published),
            Self::Name => normalize(&a.1.name).cmp(&normalize(&b.1.name)),
            Self::Tracks => {
                b.0.playlist_tracks()
                    .count()
                    .cmp(&a.0.playlist_tracks().count())
            }
        };

        by_field.then_with(|| a.0.dir.cmp(&b.0.dir))
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
