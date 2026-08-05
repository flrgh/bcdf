use crate::bandcamp::FEED_URL;
use rss::Channel;

pub(crate) async fn urls(client: &reqwest::Client) -> anyhow::Result<Vec<String>> {
    let content = client
        .execute(client.get(FEED_URL).build()?)
        .await?
        .bytes()
        .await?;

    Ok(Channel::read_from(&content[..])?
        .into_items()
        .drain(..)
        .map(|item| item.link)
        .filter(Option::is_some)
        .flatten()
        .collect())
}
