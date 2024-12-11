use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CACHE_CONTROL, PRAGMA, REFERER};
use reqwest::Client;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0";
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

    Client::builder()
        .user_agent(USER_AGENT)
        .default_headers(headers)
        .build()
        .expect("unreachable!")
}
