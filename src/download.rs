use std::path::PathBuf;

use futures::stream::StreamExt;
use id3::frame::Content;
use id3::{Frame, Tag, TagLike, Version};
use reqwest::Client;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;

pub(crate) async fn download(state: &crate::state::State) {
    let mut set: JoinSet<anyhow::Result<()>> = JoinSet::new();

    let client = Client::new();

    for track in &state.blog_info.tracks {
        let track = track.clone();

        let Some(url) = track.download_url.clone() else {
            continue;
        };

        let client = client.clone();
        let path = state.dirname().join(track.mp3_filename());

        set.spawn(async move {
            if !path.is_file() {
                println!("downloading {} -> {}", track.title, url);

                let res = client.execute(client.get(url).build()?).await?;

                match res.status().as_u16() {
                    200 => {}
                    status => {
                        anyhow::bail!("non-200 status: {status}");
                    }
                }

                let mut fh = tokio::fs::File::create(path.clone()).await?;
                let mut bytes = res.bytes_stream();
                while let Some(bytes) = bytes.next().await {
                    let bytes = bytes?;
                    fh.write_all(bytes.as_ref()).await?;
                }

                println!("finished downloading {}", track.title);
            }

            println!("tagging {}", track.title);

            let mut tag = Tag::new();
            tag.set_title(&track.title);
            tag.set_album(&track.album.title);
            tag.set_artist(&track.artist.name);
            tag.set_album_artist(&track.artist.name);
            tag.set_track(track.number as u32);

            fn set_tag(t: &mut Tag, name: &str, value: &Option<String>) {
                if let Some(value) = value {
                    t.add_frame(id3::frame::ExtendedText {
                        description: name.to_string(),
                        value: value.clone(),
                    });
                }
            }

            set_tag(&mut tag, "bandcamp_track_id", &track.bandcamp_track_id);
            set_tag(&mut tag, "spotify_track_id", &track.spotify_id);
            set_tag(
                &mut tag,
                "bandcamp_playlist_track_number",
                &Some(track.bandcamp_playlist_track_number.to_string()),
            );

            set_tag(&mut tag, "bandcamp_artist_id", &track.artist.bandcamp_id);
            set_tag(&mut tag, "bandcamp_artist_url", &track.artist.bandcamp_url);
            set_tag(&mut tag, "spotify_artist_id", &track.artist.spotify_id);

            set_tag(&mut tag, "bandcamp_album_id", &track.album.bandcamp_id);
            set_tag(&mut tag, "bandcamp_album_url", &track.album.bandcamp_url);
            set_tag(&mut tag, "spotify_album_id", &track.album.spotify_id);

            tag.write_to_path(path, Version::Id3v24)?;

            Ok(())
        });
    }

    while let Some(res) = set.join_next().await {
        if let Err(e) = res.unwrap() {
            println!("download failed: {}", e);
        }
    }
}
