mod bandcamp;
mod spotify;
mod state;
mod types;
mod download;
mod util;

use tokio::io::AsyncWriteExt;
use futures::stream::{Stream, StreamExt};


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = "https://daily.bandcamp.com/acid-test/acid-test-march-2024";
    let info = bandcamp::BlogInfo::try_from_url(url).await?;

    println!("{}", info.url);

    //println!("{:#?}", info);

    let state = state::State::try_get_or_create(info).unwrap();
    download::download(&state).await;

    //let client = spotify::connect().await;
    //spotify::search(&client, &state.blog_info.tracks[3]).await;

    Ok(())
}
