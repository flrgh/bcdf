mod bandcamp;
mod cli;
mod download;
mod feed;
mod http;
mod log;
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
    cli::run().await
}
