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
    for t in &state.blog_info.tracks {
        if t.download_url.is_none() {
            continue;
        }

        dbg!(&t);
        let url = t.download_url.clone().unwrap();

        println!("downloading {} -> {}", t.title, url);

        let res = reqwest::get(url)
            .await
            .unwrap();

        if !res.status().is_success() {
            continue;
        }

        let path = state.dirname().join(t.filename().with_extension("mp3"));

        let mut fh = tokio::fs::File::create(path).await.unwrap();
        let mut bytes = res.bytes_stream();
        while let Some(bytes) = bytes.next().await {
            let bytes = bytes.unwrap();
            fh.write_all(bytes.as_ref()).await.unwrap();
        }
    }

    //let client = spotify::connect().await;
    //spotify::search(&client, &state.blog_info.tracks[3]).await;
}
