use clap::Subcommand;
use comfy_table::{Cell, Table};
use serde_json::{Value, json};
use std::io::Write;

use crate::cli::Format;
use crate::db::{
    CustomQueries as _, Func, IntoSimpleExpr, Order, QueryOrder as _, Select, col, entity, full::*,
    reverse, traits::*,
};
use crate::list::{
    self, ColumnSpec,
    ColumnSpec::{Num, Pin, Plain},
    Filters,
};
use crate::query::TrackQuery;

/// View and manage tracks
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
            Command::Match(match_) => match_.exec(app).await,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List tracks
    Ls(List),

    /// Show a single track
    Show(Show),

    /// Attempt to match (or re-match with -f|--force) a Bandcamp track to a Spotify track
    Match(Match),
}

#[derive(clap::Args, Debug)]
pub(crate) struct List {
    /// Select tracks matching this text, or <post>/<number>
    #[arg(value_name = "QUERY")]
    query: Option<TrackQuery>,

    #[command(flatten)]
    sort: Sort,

    #[command(flatten)]
    filters: Filters,

    #[arg(short, long, value_enum, default_value = "table")]
    output: Format,
}

impl List {
    async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let rows: Vec<PostTrack> = PostTrack::select()
            .apply(|sel| self.filters.apply_timespec(sel, col::Post::PublishedAt))
            .apply(self.query)
            .apply(self.sort)
            .limit_if(self.filters.limit())
            .into_partial_model()
            .all(&app.db)
            .await?;

        let mut out = std::io::stdout().lock();

        match self.output {
            Format::Table => {
                let mut header = CHILD_HEADER;
                header[0] = ("Track", Pin);

                let mut table = list::table(header);
                for row in &rows {
                    let mut cells = row.child_cells();
                    cells[0] = Cell::new(row.short_id());
                    table.add_row(cells);
                }

                writeln!(out, "{table}")?;
            }
            Format::Json => {
                let values: Vec<Value> = rows.iter().map(|row| row.json()).collect();
                serde_json::to_writer_pretty(&mut out, &values)?;
                writeln!(out)?;
            }
        };

        Ok(())
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct Show {
    /// Select the track matching this text, or <post>/<number>
    #[arg(value_name = "QUERY")]
    query: TrackQuery,

    #[arg(short, long, value_enum, default_value = "table")]
    output: Format,
}

impl Show {
    async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let row = PostTrack::find_unique(&app.db, &self.query).await?;

        let mut out = std::io::stdout().lock();
        match self.output {
            Format::Table => {
                let track = &row.track.track;

                let secs = track.duration as u64;
                let short_id = row.short_id();
                let number = row.post_track.post_track_number.to_string();
                let duration = format!("{}:{:02}", secs / 60, secs % 60);

                let mut table = list::detail_table();
                table.add_rows([
                    ["short_id", short_id.as_str()],
                    ["post", row.post.dir.as_str()],
                    ["number", number.as_str()],
                    ["title", track.title.as_str()],
                    ["artist", row.artist()],
                    ["album_artist", row.track.artist.name.as_str()],
                    ["album", row.track.release.title.as_str()],
                    ["duration", duration.as_str()],
                    ["bandcamp_id", track.id.as_str()],
                    [
                        "spotify_id",
                        list::short_spotify_id(opt_string(&track.spotify_id)),
                    ],
                    [
                        "spotify_playlist_id",
                        list::short_spotify_id(opt_string(&row.post_track.spotify_playlist_id)),
                    ],
                    ["file", opt_string(&row.post_track.filename)],
                    ["download_url", opt_string(&track.download_url)],
                ]);

                writeln!(out, "{table}")?;
            }
            Format::Json => {
                serde_json::to_writer_pretty(&mut out, &row.json())?;
                writeln!(out)?;
            }
        };

        Ok(())
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct Match {
    /// Select the track matching this text, or <post>/<number>
    #[arg(value_name = "QUERY")]
    query: TrackQuery,

    #[arg(short, long, default_value_t = false)]
    force: bool,

    #[arg(short = 'n', long, default_value_t = false)]
    dry_run: bool,
}

fn fmt_score(score: &Option<f64>) -> String {
    match score {
        Some(v) => v.to_string(),
        None => "unknown".to_string(),
    }
}

impl Match {
    async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let mut row = PostTrack::find_unique(&app.db, &self.query).await?;

        let old_id = row.track.track.spotify_id.clone();
        let old_score = row.track.track.spotify_match_score;

        if let (Some(id), false) = (&old_id, self.force) {
            println!(
                "unchanged: already matched ('{} - {}' => {}, score: {})",
                row.artist(),
                row.track.track.title,
                id,
                fmt_score(&old_score)
            );
            return Ok(());
        };

        let spotify = crate::spotify::connect().await?;
        row.track.track.spotify_id = None;
        row.track.track.spotify_match_score = None;

        let (new_id, new_score) = match spotify.search(&row.track).await? {
            Some((id, score)) => (Some(id), Some(score)),
            None => (None, None),
        };

        let update = match (&old_id, &new_id) {
            (Some(old), Some(new)) => {
                if old == new {
                    println!("unchanged: {old} is still the best spotify track match");
                    if old_score != new_score {
                        println!(
                            "match score will be updated from {} -> {}",
                            fmt_score(&old_score),
                            fmt_score(&new_score)
                        );
                        true
                    } else {
                        false
                    }
                } else {
                    println!(
                        "changed: {old} (score: {}) => {new} (score: {})",
                        fmt_score(&old_score),
                        fmt_score(&new_score)
                    );
                    true
                }
            }
            (None, None) => {
                println!("no new spotify match found :(");
                false
            }
            (None, Some(new)) => {
                println!("new match: {new} (score: {})", fmt_score(&new_score));
                true
            }
            (Some(old), None) => {
                println!(
                    "changed: {old} (score: {}) will be removed. This might mean the track no longer exists on Spotify or that updated search criteria determined it to no longer be a suitable match",
                    fmt_score(&old_score)
                );
                true
            }
        };

        if update {
            if self.dry_run {
                println!("dry run: no database changes made");
            } else {
                row.track.track.spotify_id = new_id;
                row.track.track.spotify_match_score = new_score;
                app.db.update_track(&row.track.track).await?;
            }
        }

        Ok(())
    }
}

#[derive(clap::Args, Debug, Clone, Copy)]
struct Sort {
    #[arg(long = "sort", value_enum, default_value = "post")]
    field: Field,

    /// Reverse the sort order
    #[arg(short, long)]
    reverse: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum Field {
    /// Sort by newest post, then track number
    Post,
    /// Sort by track title, case-insensitive
    Title,
    /// Sort by track artist name, case-insensitive
    Artist,
    /// Sort by track album name, case-insensitive
    Album,
}

impl ApplyTo<Select<entity::PostTrack>> for Sort {
    fn apply_to(self, sel: Select<entity::PostTrack>) -> Select<entity::PostTrack> {
        let (field, order) = match self.field {
            Field::Post => (col::Post::PublishedAt.into_simple_expr(), Order::Desc),
            Field::Title => (
                Func::lower(col::Track::Title.into_simple_expr()).into_simple_expr(),
                Order::Asc,
            ),
            Field::Artist => (
                Func::lower(col::Artist::Name.into_simple_expr()).into_simple_expr(),
                Order::Asc,
            ),
            Field::Album => (
                Func::lower(col::Release::Title.into_simple_expr()).into_simple_expr(),
                Order::Asc,
            ),
        };

        sel.order_by(field, reverse(order, self.reverse))
            .order_by(
                col::PostShortId::Id.into_simple_expr(),
                reverse(Order::Desc, self.reverse),
            )
            .order_by(
                col::PostTrack::PostTrackNumber.into_simple_expr(),
                reverse(Order::Asc, self.reverse),
            )
    }
}

const CHILD_HEADER: [(&str, ColumnSpec); 7] = [
    ("#", Num),
    ("Title", Plain),
    ("Artist", Plain),
    ("Album", Plain),
    ("Dur", Plain),
    ("Dl", Plain),
    ("Spotify", Pin),
];

pub(crate) fn child_table<'a>(tracks: impl IntoIterator<Item = &'a PostTrack>) -> Table {
    let mut table = list::table(CHILD_HEADER);
    for track in tracks {
        table.add_row(track.child_cells());
    }
    table
}

impl PostTrack {
    pub(crate) fn short_id(&self) -> String {
        format!(
            "{}.{:02}",
            self.post_short_id.id, self.post_track.post_track_number
        )
    }

    pub(crate) fn artist(&self) -> &str {
        self.track
            .track
            .credited_artist
            .as_deref()
            .unwrap_or(self.track.artist.name.as_str())
    }

    pub(crate) fn child_cells(&self) -> Vec<Cell> {
        let track = &self.track.track;
        let secs = track.duration as u64;

        vec![
            Cell::new(self.post_track.post_track_number),
            Cell::new(&track.title),
            Cell::new(self.artist()),
            Cell::new(&self.track.release.title),
            Cell::new(format!("{}:{:02}", secs / 60, secs % 60)),
            Cell::new(if self.post_track.filename.is_some() {
                "y"
            } else {
                ""
            }),
            Cell::new(list::short_spotify_id(opt_string(&track.spotify_id))),
        ]
    }

    pub(crate) fn child_json(&self) -> Value {
        let track = &self.track.track;

        json!({
            "title": track.title,
            "artist": self.track.artist,
            "credited_artist": track.credited_artist,
            "release": self.track.release,
            "duration_secs": track.duration,
            "post_track_number": self.post_track.post_track_number,
            "album_track_number": track.release_track_number,
            "bandcamp_id": track.id,
            "download_url": track.download_url,
            "spotify": track.spotify_id.as_ref().map(|id| json!({
                "id": id,
                "match_score": track.spotify_match_score,
                "playlist_id": self.post_track.spotify_playlist_id,
            })),
            // relative to the data directory, and what the database records
            // rather than what is on disk
            "file": self.post_track.filename.as_ref().map(|name| json!({
                "name": name,
                "path": format!("{}/{}", self.post.dir, name),
            })),
        })
    }

    fn json(&self) -> Value {
        let mut value = self.child_json();
        value["short_id"] = json!(self.short_id());
        value["post"] = self.post.json_summary(&self.post_short_id.id);
        value
    }
}

fn opt_string(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or_default()
}
