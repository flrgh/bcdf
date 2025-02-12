use crate::http;
use crate::metrics;
use futures::stream::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;

pub(crate) async fn download(state: &crate::state::State) {
    let mut set: JoinSet<anyhow::Result<()>> = JoinSet::new();

    let client = http::client();

    for track in &state.tracks {
        let track = track.clone();

        let Some(url) = track.download_url.clone() else {
            tracing::debug!(track.title, "SKIP: no download url");
            continue;
        };

        let path = state.dirname().join(track.mp3_filename());

        if path.is_file() {
            tracing::debug!(track.title, "SKIP: exists");
            continue;
        }

        let client = client.clone();

        set.spawn(async move {
            tracing::info!(track.title, "downloading");

            let req = client.get(url).build()?;
            let res = client.execute(req).await?;

            match res.status().as_u16() {
                200 => {}
                status => {
                    let body = res.text().await.ok();
                    tracing::error!(track.title, status, body, "download failed");

                    anyhow::bail!("non-200 status: {status}");
                }
            }

            let mut fh = tokio::fs::File::create(path.clone()).await?;
            let mut bytes = res.bytes_stream();
            while let Some(bytes) = bytes.next().await {
                let bytes = bytes?;
                fh.write_all(bytes.as_ref()).await?;
            }

            tracing::debug!(track.title, "finished downloading");
            metrics::inc(metrics::TracksDownloaded, 1);
            Ok(())
        });
    }

    while let Some(res) = set.join_next().await {
        match res {
            Ok(join_res) => {
                if let Err(error) = join_res {
                    tracing::error!(?error, "download failed");
                }
            }
            Err(error) => {
                tracing::error!(?error, "download failed");
            }
        }
    }
}
