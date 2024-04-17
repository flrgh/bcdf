use crate::bandcamp::FEED_URL;
use rss::Channel;

pub(crate) async fn urls() -> anyhow::Result<Vec<String>> {
    let content = reqwest::get(FEED_URL).await?.bytes().await?;

    Ok(Channel::read_from(&content[..])?
        .into_items()
        .drain(..)
        .map(|item| item.link)
        .filter(Option::is_some)
        .flatten()
        .collect())
}
