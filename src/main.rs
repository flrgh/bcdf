mod bandcamp;
mod download;
mod search;
mod spotify;
mod state;
mod types;
mod util;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let urls: Vec<String> = std::env::args().skip(1).collect();

    if urls.is_empty() {
        return Ok(());
    }

    let spotify = spotify::connect().await?;
    for url in urls {
        let info = bandcamp::BlogInfo::try_from_url(&url).await?;

        println!("{}", info.url);

        let mut state = state::State::try_get_or_create(info)?;
        download::download(&state).await;

        spotify.exec(&mut state).await?;
        state.save()?;
    }

    Ok(())
}
