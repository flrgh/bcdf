mod bandcamp;
mod spotify;
mod types;

#[tokio::main]
async fn main() {
    let html = include_str!("../test/content.html");
    let info = bandcamp::BlogInfo::from_html(html);

    //println!("{:#?}", info);

    let client = spotify::connect().await;
    spotify::search(&client, &info.tracks[3]).await;
}
