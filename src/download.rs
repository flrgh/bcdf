use crate::http;
use crate::metrics;
use crate::store::Store;
use crate::types::BlogPost;
use anyhow::Context;
use futures::stream::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;

pub(crate) async fn download(store: &Store, post: &mut BlogPost) -> anyhow::Result<()> {
    let dir = store.post_dir(post);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating post directory {dir:?}"))?;

    let mut set: JoinSet<anyhow::Result<(usize, String)>> = JoinSet::new();
    let mut downloaded = Vec::new();

    let client = http::client();

    for track in &post.tracks {
        let number = track.post_track_number;

        if store.is_downloaded(post, track) {
            tracing::debug!(track.title, "SKIP: exists");
            continue;
        }

        let filename = track.derive_filename();
        let path = dir.join(&filename);

        if path.is_file() {
            tracing::debug!(track.title, "SKIP: exists, recording it");
            downloaded.push((number, filename));
            continue;
        }

        let Some(url) = track.download_url.clone() else {
            tracing::debug!(track.title, "SKIP: no download url");
            continue;
        };

        let client = client.clone();
        let title = track.title.clone();

        set.spawn(async move {
            tracing::info!(title, "downloading");

            let req = client.get(url).build()?;
            let res = client.execute(req).await?;

            match res.status().as_u16() {
                200 => {}
                status => {
                    let body = res.text().await.ok();
                    tracing::error!(title, status, body, "download failed");

                    anyhow::bail!("non-200 status: {status}");
                }
            }

            let mut fh = tokio::fs::File::create(&path).await?;
            let mut bytes = res.bytes_stream();
            while let Some(bytes) = bytes.next().await {
                let bytes = bytes?;
                fh.write_all(bytes.as_ref()).await?;
            }

            tracing::debug!(title, "finished downloading");
            metrics::inc(metrics::TracksDownloaded, 1);
            Ok((number, filename))
        });
    }

    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(written)) => downloaded.push(written),
            Ok(Err(error)) => tracing::error!(?error, "download failed"),
            Err(error) => tracing::error!(?error, "download failed"),
        }
    }

    for (number, filename) in downloaded {
        if let Err(error) = store.set_track_filename(&post.url, number, &filename) {
            tracing::error!(?error, url = post.url, number, "recording mp3 failed");
            continue;
        }

        if let Some(track) = post
            .tracks
            .iter_mut()
            .find(|t| t.post_track_number == number)
        {
            track.filename = Some(filename);
        }
    }

    Ok(())
}
