mod bandcamp;
mod spotify;
mod state;
mod types;
mod download;
mod util;

use tokio::io::AsyncWriteExt;
use futures::stream::{Stream, StreamExt};


#[tokio::main]
async fn main() {
    let html = include_str!("../test/content.html");
    let info = bandcamp::BlogInfo::from_html(html);

    println!("{}", info.url);

    //println!("{:#?}", info);

    let state = state::State::try_get_or_create(info).unwrap();
    download::download(&state).await;

    //let client = spotify::connect().await;
    //spotify::search(&client, &state.blog_info.tracks[3]).await;
}
