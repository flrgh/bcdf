use chrono::SecondsFormat;
use clap::Subcommand;
use comfy_table::Cell;
use sea_orm::sea_query::{Expr, Func};
use sea_orm::{IntoSimpleExpr, Order, QueryOrder, Select};
use serde_json::{Value, json};
use std::io::Write;

use crate::cli::Format;
use crate::db::{
    col, entity,
    traits::*,
    views::{PlaylistDetail, PlaylistTrackDetail},
};
use crate::list::{
    self,
    ColumnSpec::{Num, Pin, Plain},
    Filters,
};
use crate::query::Query;

/// View and manage Spotify playlists
#[derive(clap::Args, Debug)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub(crate) async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        match self.command {
            Command::Ls(ls) => ls.exec(app).await,
            Command::Show(show) => show.exec(app).await,
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

#[derive(clap::Args, Debug, Clone, Copy)]
struct Sort {
    #[arg(long = "sort", value_enum, default_value = "published")]
    field: Field,

    /// Reverse the sort order
    #[arg(short, long)]
    reverse: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum Field {
    /// Sort by newest post
    Published,
    /// Sort by playlist name, case-insensitive
    Name,
    /// Sort by track count, most first
    Tracks,
}

impl ApplyTo<Select<entity::Playlist>> for Sort {
    fn apply_to(self, sel: Select<entity::Playlist>) -> Select<entity::Playlist> {
        let (field, order) = match self.field {
            Field::Published => (col::Post::PublishedAt.into_simple_expr(), Order::Desc),
            Field::Name => (
                Func::lower(col::Playlist::Name.into_simple_expr()).into_simple_expr(),
                Order::Asc,
            ),
            Field::Tracks => (Expr::col("tracks"), Order::Desc),
        };

        sel.order_by(field, reverse(order, self.reverse)).order_by(
            col::PostShortId::Id.into_simple_expr(),
            reverse(Order::Desc, self.reverse),
        )
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct List {
    /// Select playlists matching this text
    #[arg(value_name = "QUERY")]
    query: Option<Query>,

    #[command(flatten)]
    sort: Sort,

    #[command(flatten)]
    filters: Filters,

    #[arg(short, long, value_enum, default_value = "table")]
    output: Format,
}

impl List {
    async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let select = PlaylistDetail::select()
            .apply(|sel| self.filters.apply_timespec(sel, col::Post::PublishedAt))
            .apply(self.query)
            .apply(self.sort)
            .limit_if(self.filters.limit());

        let rows: Vec<PlaylistDetail> = select.into_partial_model().all(&app.db).await?;

        let mut out = std::io::stdout().lock();

        match self.output {
            Format::Table => {
                let mut table = list::table([
                    ("Id", Pin),
                    ("Name", Plain),
                    ("Tracks", Num),
                    ("Post", Plain),
                ]);

                for row in rows {
                    table.add_row(vec![
                        Cell::new(list::short_spotify_id(&row.id)),
                        Cell::new(&row.name),
                        Cell::new(row.tracks),
                        Cell::new(&row.short_id.id),
                    ]);
                }

                writeln!(out, "{table}")?;
            }
            Format::Json => {
                let values: Vec<Value> = rows
                    .iter()
                    .map(|row| {
                        json!({
                            "id": row.id,
                            "name": row.name,
                            "tracks": row.tracks,
                            "post": row.post.json_summary(&row.short_id.id),
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

impl PlaylistTrackDetail {
    fn artist(&self) -> &str {
        self.track
            .credited_artist
            .as_deref()
            .unwrap_or(self.artist.name.as_str())
    }

    fn cells(&self) -> Vec<Cell> {
        let secs = self.track.duration as u64;

        vec![
            Cell::new(self.post_track_number),
            Cell::new(&self.track.title),
            Cell::new(self.artist()),
            Cell::new(&self.release.title),
            Cell::new(format!("{}:{:02}", secs / 60, secs % 60)),
            Cell::new(if self.filename.is_some() { "y" } else { "" }),
            Cell::new(list::short_spotify_id(
                self.track.spotify_id.as_deref().unwrap_or_default(),
            )),
        ]
    }

    fn child_json(&self, post_dir: &str) -> Value {
        let track = &self.track;

        json!({
            "title": track.title,
            "artist": { "name": self.artist() },
            "album_artist": self.artist,
            "album": self.release,
            "duration_secs": track.duration,
            "post_track_number": self.post_track_number,
            "album_track_number": track.release_track_number,
            "bandcamp_id": track.id,
            "download_url": track.download_url,
            "spotify": track.spotify_id.as_ref().map(|id| json!({
                "id": id,
                "match_score": track.spotify_match_score,
            })),
            "file": self.filename.as_ref().map(|name| json!({
                "name": name,
                "path": format!("{post_dir}/{name}"),
            })),
        })
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
    async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let Some(row) = PlaylistDetail::select_unique(&self.query)
            .into_partial_model::<PlaylistDetail>()
            .one(&app.db)
            .await?
        else {
            anyhow::bail!("no playlist matched {}", self.query);
        };

        let tracks: Vec<PlaylistTrackDetail> = PlaylistTrackDetail::select(&row.id)
            .into_partial_model::<PlaylistTrackDetail>()
            .all(&app.db)
            .await?;

        let mut out = std::io::stdout().lock();
        match self.output {
            Format::Table => {
                let published = row
                    .post
                    .published_at
                    .to_rfc3339_opts(SecondsFormat::Secs, true);
                let count = tracks.len().to_string();

                let mut table = list::detail_table();
                table.add_rows([
                    ["id", list::short_spotify_id(&row.id)],
                    ["name", row.name.as_str()],
                    ["post", row.short_id.id.as_str()],
                    ["published", published.as_str()],
                    ["tracks", count.as_str()],
                ]);

                let mut child = list::table([
                    ("#", Num),
                    ("Title", Plain),
                    ("Artist", Plain),
                    ("Album", Plain),
                    ("Dur", Plain),
                    ("Dl", Plain),
                    ("Spotify", Pin),
                ]);

                for track in &tracks {
                    child.add_row(track.cells());
                }

                writeln!(out, "{table}")?;
                writeln!(out, "{child}")?;
            }
            Format::Json => {
                let tracks: Vec<Value> = tracks
                    .iter()
                    .map(|track| track.child_json(&row.post.dir))
                    .collect();

                serde_json::to_writer_pretty(
                    &mut out,
                    &json!({
                        "id": row.id,
                        "name": row.name,
                        "post": row.post.json_summary(&row.short_id.id),
                        "tracks": tracks,
                    }),
                )?;

                writeln!(out)?;
            }
        };

        Ok(())
    }
}
