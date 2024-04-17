use futures::stream::StreamExt;
use reqwest::Client;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;

#[tracing::instrument]
pub(crate) async fn download(state: &crate::state::State) {
    let mut set: JoinSet<anyhow::Result<()>> = JoinSet::new();

    let client = Client::new();

    for track in &state.tracks {
        let track = track.clone();

        let Some(url) = track.download_url.clone() else {
            tracing::debug!(?track, "SKIP: no download url");
            continue;
        };

        let client = client.clone();
        let path = state.dirname().join(track.mp3_filename());

        if path.is_file() {
            tracing::debug!(?track, "SKIP: exists");
            continue;
        }

        set.spawn(async move {
            tracing::info!(?track, "downloading");

            let res = client.execute(client.get(url).build()?).await?;

            match res.status().as_u16() {
                200 => {}
                status => {
                    let body = res.text().await.ok();
                    tracing::error!(?track, status, body, "download failed");

                    anyhow::bail!("non-200 status: {status}");
                }
            }

            let mut fh = tokio::fs::File::create(path.clone()).await?;
            let mut bytes = res.bytes_stream();
            while let Some(bytes) = bytes.next().await {
                let bytes = bytes?;
                fh.write_all(bytes.as_ref()).await?;
            }

            tracing::debug!(?track, "finished downloading");
            Ok(())
        });
    }

    while let Some(res) = set.join_next().await {
        if let Err(e) = res.unwrap() {
            tracing::error!(error = ?e, "download failed");
        }
    }
}
