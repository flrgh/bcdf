use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CACHE_CONTROL, PRAGMA, REFERER};
use reqwest::Client;

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

pub(crate) fn client() -> reqwest::Client {
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
}
