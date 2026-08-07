mod bandcamp;
mod cli;
mod download;
mod feed;
mod http;
mod metrics;
mod mp3;
mod rename;
mod run;
mod search;
mod spotify;
mod store;
mod tag;
mod types;
mod util;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    cli::run().await
}
