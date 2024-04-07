use tokio::io::AsyncWriteExt;
use futures::stream::StreamExt;

pub(crate) async fn download(state: &crate::state::State) {
    for t in &state.blog_info.tracks {
        let Some(url) = &t.download_url else {
            continue;
        };

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
}
