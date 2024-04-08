mod bandcamp;
mod spotify;
mod state;
mod types;
mod download;
mod util;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let urls = vec![
        "https://daily.bandcamp.com/scene-report/bristol-avant-folk-music-scene-report",
        "https://daily.bandcamp.com/acid-test/acid-test-march-2024",
        "https://daily.bandcamp.com/best-contemporary-classical/the-best-contemporary-classical-music-on-bandcamp-march-2024",
        "https://daily.bandcamp.com/best-of-2024/the-best-albums-of-winter-2024",
    ];

    for url in urls {
        let info = bandcamp::BlogInfo::try_from_url(url).await?;

        println!("{}", info.url);

        let state = state::State::try_get_or_create(info)?;
        download::download(&state).await;
    }

    Ok(())
}
