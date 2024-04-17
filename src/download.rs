use futures::stream::StreamExt;
use reqwest::Client;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;

pub(crate) async fn download(state: &crate::state::State) {
    let mut set: JoinSet<anyhow::Result<()>> = JoinSet::new();

    let client = Client::new();

    for track in &state.tracks {
        let track = track.clone();

        let Some(url) = track.download_url.clone() else {
            continue;
        };

        let client = client.clone();
        let path = state.dirname().join(track.mp3_filename());

        if path.is_file() {
            continue;
        }

        set.spawn(async move {
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
            Ok(())
        });
    }

    while let Some(res) = set.join_next().await {
        if let Err(e) = res.unwrap() {
            println!("download failed: {}", e);
        }
    }
}
