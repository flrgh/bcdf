mod bandcamp;
mod download;
mod feed;
mod search;
mod spotify;
mod state;
mod tag;
mod types;
mod util;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let urls = feed::urls().await?;

    if urls.is_empty() {
        return Ok(());
    }

    let spotify = spotify::connect().await?;

    for url in urls {
        let info = bandcamp::BlogInfo::try_from_url(&url).await?;

        let mut state = state::State::try_get_or_create(info)?;

        spotify.exec(&mut state).await?;
        state.save()?;

        download::download(&state).await;
        state.save()?;

        tag::tag(&state).await?;
    }

    Ok(())
}
