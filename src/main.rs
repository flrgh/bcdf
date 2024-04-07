mod bandcamp;
mod spotify;
mod state;
mod types;
mod download;
mod util;

async fn download(url: &str) -> anyhow::Result<()> {
    let info = bandcamp::BlogInfo::try_from_url(url).await?;

    println!("{}", info.url);

    let state = state::State::try_get_or_create(info).unwrap();
    download::download(&state).await;

    Ok(())
}


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    download("https://daily.bandcamp.com/acid-test/acid-test-march-2024").await?;
    download("https://daily.bandcamp.com/best-contemporary-classical/the-best-contemporary-classical-music-on-bandcamp-march-2024").await?;
    download("https://daily.bandcamp.com/best-of-2024/the-best-albums-of-winter-2024").await?;

    Ok(())
}
