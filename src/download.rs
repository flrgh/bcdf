use anyhow::Context;
use futures::stream::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;

use crate::db::{CustomQueries as _, full};
use crate::http;
use crate::metrics;

impl crate::App {
    pub(crate) async fn download(&self, data: &mut full::Post) -> anyhow::Result<()> {
        let dir = self.path(&data.post.dir);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating post directory {dir:?}"))?;

        let mut set: JoinSet<anyhow::Result<(u32, String)>> = JoinSet::new();
        let mut downloaded = Vec::new();

        let client = http::client();

        for track in &data.tracks {
            if track
                .post_track
                .filename
                .as_ref()
                .is_some_and(|fname| dir.join(fname).exists())
            {
                tracing::debug!(track.track.track.title, "SKIP: exists");
                continue;
            }

            let number = track.post_track.post_track_number;

            let filename = track.derive_filename();
            let path = dir.join(&filename);

            if path.is_file() {
                tracing::debug!(track.track.track.title, "SKIP: exists, recording it");
                downloaded.push((number, filename));
                continue;
            }

            let Some(url) = track.track.track.download_url.clone() else {
                tracing::debug!(track.track.track.title, "SKIP: no download url");
                continue;
            };

            let client = client.clone();
            let title = track.track.track.title.clone();

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

                // FIXME: download to temp file and rename into place on success
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
            if let Some(track) = data
                .tracks
                .iter_mut()
                .find(|t| t.post_track.post_track_number == number)
            {
                let old = track.post_track.filename.take();
                track.post_track.filename = Some(filename);
                match self.db.update_post_track(&track.post_track).await {
                    Err(error) => {
                        tracing::error!(
                            ?error,
                            url = data.post.url,
                            number,
                            "recording mp3 failed"
                        );
                        track.post_track.filename = old;
                    }
                    Ok(pt) => {
                        track.post_track = pt;
                    }
                }
            }
        }

        Ok(())
    }
}
