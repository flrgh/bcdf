use std::sync::LazyLock;

pub(crate) use reqwest::Client;
use reqwest::header::{CACHE_CONTROL, HeaderMap, HeaderName, HeaderValue, PRAGMA, REFERER};

const DEFAULT_USER_AGENT: &str = concat!(
    "Bandcamp Daily Blog Fetcher/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/flrgh/bcdf)"
);
const DEFAULT_HEADERS: &[(HeaderName, &str)] = &[
    (PRAGMA, "no-cache"),
    (CACHE_CONTROL, "no-cache"),
    (REFERER, "https://daily.bandcamp.com/"),
];

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    let headers = HeaderMap::from_iter(
        DEFAULT_HEADERS
            .iter()
            .map(|(name, value)| (name.clone(), HeaderValue::from_static(value))),
    );

    let user_agent = std::env::var("BCDF_USER_AGENT")
        .ok()
        .unwrap_or_else(|| DEFAULT_USER_AGENT.to_string());

    Client::builder()
        .user_agent(user_agent)
        .default_headers(headers)
        .build()
        .expect("unreachable!")
});

pub(crate) fn client() -> &'static reqwest::Client {
    &CLIENT
}
