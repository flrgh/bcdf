use chrono::SecondsFormat;
use clap::Subcommand;
use comfy_table::Cell;
use serde_json::{Value, json};
use std::io::Write;

use crate::cli::Format;
use crate::db::{
    Func, IntoSimpleExpr, Order, QueryOrder, Select, col, entity, full, model, reverse,
    traits::{Apply, ApplyTo, LimitIf},
    views,
};
use crate::list::ColumnSpec::{Num, Pin, Plain};
use crate::list::{self, Filters};
use crate::query::Query;
use crate::track;

/// View/manage bandcamp blog posts
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

#[derive(clap::Args, Debug)]
pub(crate) struct List {
    /// Select posts matching this text
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
        let select = views::PostSummary::select()
            .apply(|sel| self.filters.apply_timespec(sel, col::Post::PublishedAt))
            .apply(self.query)
            .apply(self.sort)
            .limit_if(self.filters.limit())
            .into_partial_model();

        let posts: Vec<views::PostSummary> = select.all(&app.db).await?;

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
                        Cell::new(&row.short_id.id),
                        Cell::new(row.tracks),
                        Cell::new(row.downloaded),
                        Cell::new(row.spotify),
                        Cell::new(
                            row.playlist
                                .as_ref()
                                .map(|p| list::short_spotify_id(&p.id))
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
                            "short_id": row.short_id.id,
                            "dir": post.dir,
                            "url": post.url,
                            "title": post.title,
                            "published": post.published_at,
                            "modified": post.modified_at,
                            "tracks": row.tracks,
                            "downloaded": row.downloaded,
                            "spotify": row.spotify,
                            "playlist": row.playlist,
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
    async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let Some(row) = full::Post::find_unique(&app.db, &self.query).await? else {
            anyhow::bail!("no post matched {}", self.query);
        };

        let mut out = std::io::stdout().lock();
        let post = &row.post;
        match self.output {
            Format::Table => {
                let published = post.published_at.to_rfc3339_opts(SecondsFormat::Secs, true);
                let modified = post.modified_at.to_rfc3339_opts(SecondsFormat::Secs, true);

                let tracks = format!(
                    "{} ({} downloaded, {} on spotify)",
                    row.tracks.len(),
                    row.downloaded_count(),
                    row.spotify_count()
                );

                let playlist = match &row.playlist {
                    Some(playlist) => format!(
                        "{} \u{2014} {}",
                        list::short_spotify_id(&playlist.id),
                        playlist.name
                    ),
                    None => String::new(),
                };

                let mut table = list::detail_table();
                table.add_rows([
                    ["short_id", row.short_id.id.as_str()],
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

                writeln!(out, "{}", track::child_table(&row.tracks))?;
            }
            Format::Json => {
                let tracks: Vec<Value> = row.tracks.iter().map(|row| row.child_json()).collect();

                serde_json::to_writer_pretty(&mut out, &{
                    json!({
                        "short_id": row.short_id.id,
                        "dir": post.dir,
                        "url": post.url,
                        "title": post.title,
                        "description": post.description,
                        "published": post.published_at,
                        "modified": post.modified_at,
                        "playlist": row.playlist,
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
    /// Sort by date published (newest first)
    Published,
    /// Sort by title, case-insensitive
    Title,
    /// Sort by post directory, case-insensitive
    Dir,
}

impl ApplyTo<Select<entity::Post>> for Sort {
    fn apply_to(self, sel: Select<entity::Post>) -> Select<entity::Post> {
        let (field, order) = match self.field {
            Field::Published => (col::Post::PublishedAt.into_simple_expr(), Order::Desc),
            Field::Title => (
                Func::lower(col::Post::Title.into_simple_expr()).into_simple_expr(),
                Order::Asc,
            ),
            Field::Dir => (
                Func::lower(col::Post::Dir.into_simple_expr()).into_simple_expr(),
                Order::Asc,
            ),
        };

        sel.order_by(field, reverse(order, self.reverse))
    }
}

impl model::Post {
    /// A post as it appears nested inside a track or playlist
    pub(crate) fn json_summary(&self, short_id: &str) -> Value {
        json!({
            "short_id": short_id,
            "url": self.url,
            "dir": self.dir,
            "title": self.title,
            "published": self.published_at,
        })
    }
}
