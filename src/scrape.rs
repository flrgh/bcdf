use anyhow::Context;
use comfy_table::{Cell, ContentLineStyle, LineStyle};
use sea_orm::sea_query::Func;
use serde_json::to_string as serialize;
use serde_json::{Value, map::Map};

use crate::bandcamp::{post_path, post_url};
use crate::cli::Format;
use crate::db::{
    EntityTrait as _, IntoSimpleExpr as _, Order, QueryOrder as _, Select, col, entity, reverse,
    traits::{Apply, ApplyTo, LimitIf},
    views,
};
use crate::list::ColumnSpec::*;
use crate::list::{self, Filters};
use crate::query::Query;

/// Inspect or manage BandCamp blog post scrape tasks
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
            Command::Tracks(tracks) => tracks.exec(app).await,
        }
    }
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// List recorded post scrapes
    Ls(List),

    /// Print a detailed view of a scrape result
    Show(Show),

    /// Print the scraped player data for a post's tracks
    Tracks(Tracks),
}

#[derive(clap::Args, Debug, Clone, Copy)]
struct Sort {
    #[arg(long = "sort", value_enum, default_value = "updated")]
    field: Field,

    /// Reverse the sort order
    #[arg(short, long)]
    reverse: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum Field {
    /// Sort by most recently scraped
    Updated,
    /// Sort by when the scrape was first recorded
    Created,
    /// Sort by post path, case-insensitive
    Path,
}

impl ApplyTo<Select<entity::Scrape>> for Sort {
    fn apply_to(self, sel: Select<entity::Scrape>) -> Select<entity::Scrape> {
        let (field, order) = match self.field {
            Field::Updated => (col::Scrape::UpdatedAt.into_simple_expr(), Order::Desc),
            Field::Created => (col::Scrape::CreatedAt.into_simple_expr(), Order::Desc),
            Field::Path => (
                Func::lower(col::Scrape::Url.into_simple_expr()).into_simple_expr(),
                Order::Asc,
            ),
        };

        sel.order_by(field, reverse(order, self.reverse))
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct List {
    /// Select scrapes matching this url or path
    #[arg(value_name = "QUERY")]
    query: Option<Query>,

    #[command(flatten)]
    sort: Sort,

    #[command(flatten)]
    filters: Filters,
}

impl List {
    async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let scrapes: Vec<views::ScrapeInfo> = entity::Scrape::find()
            .apply(|sel| self.filters.apply_timespec(sel, col::Scrape::UpdatedAt))
            .apply(self.query)
            .apply(self.sort)
            .limit_if(self.filters.limit())
            .into_partial_model()
            .all(&app.db)
            .await?;

        let mut tab = list::table([("path", Plain), ("createded", Plain), ("updated", Plain)]);

        for scrape in scrapes {
            tab.add_row([
                Cell::new(post_path(&scrape.url)),
                Cell::new(scrape.created_at),
                Cell::new(scrape.updated_at),
            ]);
        }

        println!("{tab}");

        Ok(())
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct Show {
    /// Scraped URL or path
    url: String,

    #[arg(short, long, value_enum, default_value = "table")]
    output: Format,
}

impl Show {
    async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let url = post_url(&self.url).into_owned();
        let scrape = entity::Scrape::find_by_id(url.clone())
            .into_partial_model::<views::ScrapeInfo>()
            .one(&app.db)
            .await?
            .with_context(|| format!("no scrape recorded for {url}"))?;

        match self.output {
            Format::Table => {
                let metadata = scrape.metadata;

                let mut tab = list::detail_table();
                tab.add_rows(&[
                    ["url", &url],
                    ["created_at", &scrape.created_at.to_string()],
                    ["updated_at", &scrape.updated_at.to_string()],
                ]);

                tab.add_row(["---", "---"]);

                let mut kvs: Vec<(String, String)> = metadata.0.into_iter().collect();
                kvs.sort_by(|a, b| a.0.cmp(&b.0));
                for (key, value) in kvs {
                    tab.add_row([key, value]);
                }

                println!("{tab}");
            }
            Format::Json => {
                println!("{}", serde_json::to_string(&scrape)?);
            }
        };

        Ok(())
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct Tracks {
    /// Scraped URL or path
    url: String,

    #[arg(short, long, value_enum, default_value = "table")]
    output: Format,
}

impl Tracks {
    async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        let url = post_url(&self.url);
        let tracks = entity::Scrape::get_player_json(&app.db, &url).await?;

        match self.output {
            Format::Table => {
                fn map_get<'a>(map: &'a Map<String, Value>, k: &str) -> &'a Value {
                    map.get(k).unwrap_or(&Value::Null)
                }

                use comfy_table::presets::UTF8_FULL_CONDENSED;
                let sub_style = UTF8_FULL_CONDENSED
                    .content_lines(ContentLineStyle::new(' ', '┆', ' '))
                    .top_border(LineStyle::new(' ', '─', '┬', ' '))
                    .bottom_border(LineStyle::none());

                let props = [
                    "featured_track_number",
                    "title",
                    "band_id",
                    "band_name",
                    "band_url",
                    "parent_tralbum_id",
                    "parent_tralbum_type",
                    "show_track",
                    "tralbum_key",
                    "tralbum_url",
                ];

                let sub_props = [
                    "track_number",
                    "track_title",
                    "track_id",
                    "album_id",
                    "artist",
                    "audio_track_duration",
                ];

                for (idx, track) in tracks.iter().enumerate() {
                    let post_track_number = idx + 1;

                    let Value::Object(track) = track else {
                        tracing::warn!(
                            "track {post_track_number} of {} is not a JSON object: {}",
                            tracks.len(),
                            track
                        );
                        continue;
                    };

                    let mut tab = list::detail_table();
                    tab.add_row([
                        "post_track_number".to_string(),
                        post_track_number.to_string(),
                    ]);

                    for name in props {
                        let name = name.to_string();
                        let value = map_get(track, &name);
                        tab.add_row([
                            name,
                            value
                                .as_str()
                                .map_or_else(|| serialize(value).unwrap(), |s| s.to_string()),
                        ]);
                    }

                    let Some(Value::Array(tracklist)) = track.get("tracklist") else {
                        println!("{tab}");
                        continue;
                    };

                    for (idx, elem) in tracklist.iter().enumerate() {
                        let idx = idx + 1;

                        let mut child = list::detail_table();
                        child.load_style(sub_style);

                        let Value::Object(elem) = elem else {
                            tracing::warn!(
                                "tracklist item {idx} of {} is not a JSON object: {}",
                                tracklist.len(),
                                elem
                            );

                            child.add_row(["json".to_string(), serialize(elem).unwrap()]);

                            tab.add_rows([[format!("track #{}", idx), child.to_string()]]);

                            continue;
                        };

                        for name in sub_props {
                            let name = name.to_string();
                            let value = map_get(elem, &name);
                            child.add_row([
                                name,
                                value
                                    .as_str()
                                    .map_or_else(|| serialize(value).unwrap(), |s| s.to_string()),
                            ]);
                        }

                        tab.add_rows([[format!("track #{}", idx), child.to_string()]]);
                    }

                    println!("{tab}");
                }
            }
            Format::Json => {
                println!("{}", serialize(&tracks)?);
            }
        };

        Ok(())
    }
}
