use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest_middleware::{ClientWithMiddleware, RequestBuilder};
use url::Url;

// application/vnd.github.raw+json is required for GitHub API to return raw
// file contents
const KPAR_ACCEPT: &str =
    "application/zip, application/octet-stream, application/vnd.github.raw+json";
const JSON_ACCEPT: &str = "application/vnd.github.raw+json, application/json, text/plain";
// application/octet-stream is included here because `.sysml`/`.kerml`
// file extensions are unusual enough that some servers are likely to
// treat them as binary data
const TEXT_ACCEPT: &str = "application/vnd.github.raw+json, text/plain, application/octet-stream";

/// For KPAR and other binary files
pub fn kpar_get_request(url: &Url) -> impl Fn(&ClientWithMiddleware) -> RequestBuilder + use<> {
    let this_url = url.clone();
    move |client: &ClientWithMiddleware| -> RequestBuilder {
        client
            .get(this_url.clone())
            .header(header::ACCEPT, KPAR_ACCEPT)
    }
}

pub fn kpar_head_request(url: &Url) -> impl Fn(&ClientWithMiddleware) -> RequestBuilder + use<> {
    let this_url = url.clone();
    move |client: &ClientWithMiddleware| -> RequestBuilder {
        client
            .head(this_url.clone())
            .header(header::ACCEPT, KPAR_ACCEPT)
    }
}

/// For JSON files
pub fn json_get_request(url: &Url) -> impl Fn(&ClientWithMiddleware) -> RequestBuilder + use<> {
    let this_url = url.clone();
    move |client: &ClientWithMiddleware| -> RequestBuilder {
        client
            .get(this_url.clone())
            .header(header::ACCEPT, JSON_ACCEPT)
    }
}

pub fn json_head_request(url: &Url) -> impl Fn(&ClientWithMiddleware) -> RequestBuilder + use<> {
    let this_url = url.clone();
    move |client: &ClientWithMiddleware| -> RequestBuilder {
        client
            .head(this_url.clone())
            .header(header::ACCEPT, JSON_ACCEPT)
    }
}

/// For all text files that are not JSON
pub fn text_get_request(url: &Url) -> impl Fn(&ClientWithMiddleware) -> RequestBuilder + use<> {
    let this_url = url.clone();
    move |client: &ClientWithMiddleware| -> RequestBuilder {
        client
            .get(this_url.clone())
            .header(header::ACCEPT, TEXT_ACCEPT)
    }
}

pub fn create_reqwest_client() -> reqwest_middleware::ClientWithMiddleware {
    const UA: &str = concat!("sysand/", env!("CARGO_PKG_VERSION"));
    let mut headers = HeaderMap::new();
    headers.insert(header::USER_AGENT, HeaderValue::from_static(UA));

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap();

    reqwest_middleware::ClientBuilder::new(client).build()
}
