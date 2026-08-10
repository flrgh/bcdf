mod bandcamp;
mod cli;
mod download;
mod feed;
mod http;
mod list;
mod log;
mod metrics;
mod mp3;
mod playlist;
mod post;
mod query;
mod rename;
mod scan;
mod search;
mod spotify;
mod store;
mod tag;
mod track;
mod types;
mod util;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::new();
    cli.exec().await
}
