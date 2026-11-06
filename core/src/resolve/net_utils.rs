// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{error::Error, fmt::Display, hash::BuildHasher as _, net::IpAddr, time::Duration};

use reqwest::header;
use reqwest_middleware::{ClientWithMiddleware, RequestBuilder};
use url::Url;

use crate::redact::RedactedUrl;

// application/vnd.github.raw is required for GitHub API to return raw
// file contents
const KPAR_ACCEPT: &str = "application/zip, application/octet-stream, application/vnd.github.raw";
const JSON_ACCEPT: &str = "application/vnd.github.raw, application/json, text/plain";
// application/octet-stream is included here because `.sysml`/`.kerml`
// file extensions are unusual enough that some servers are likely to
// treat them as binary data
const TEXT_ACCEPT: &str = "application/vnd.github.raw, text/plain, application/octet-stream";

/// For KPAR and other binary files
pub fn kpar_get_request(url: impl Into<Url>) -> impl Fn(&ClientWithMiddleware) -> RequestBuilder {
    let this_url = url.into();
    move |client: &ClientWithMiddleware| -> RequestBuilder {
        client
            .get(this_url.clone())
            .header(header::ACCEPT, KPAR_ACCEPT)
    }
}

/// For JSON files
pub fn json_get_request(url: impl Into<Url>) -> impl Fn(&ClientWithMiddleware) -> RequestBuilder {
    let this_url = url.into();
    move |client: &ClientWithMiddleware| -> RequestBuilder {
        client
            .get(this_url.clone())
            .header(header::ACCEPT, JSON_ACCEPT)
    }
}

pub fn json_head_request(url: impl Into<Url>) -> impl Fn(&ClientWithMiddleware) -> RequestBuilder {
    let this_url = url.into();
    move |client: &ClientWithMiddleware| -> RequestBuilder {
        client
            .head(this_url.clone())
            .header(header::ACCEPT, JSON_ACCEPT)
    }
}

/// For all text files that are not JSON
pub fn text_get_request(url: impl Into<Url>) -> impl Fn(&ClientWithMiddleware) -> RequestBuilder {
    let this_url = url.into();
    move |client: &ClientWithMiddleware| -> RequestBuilder {
        client
            .get(this_url.clone())
            .header(header::ACCEPT, TEXT_ACCEPT)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("failed to build reqwest HTTP client")]
pub struct ReqwestClientBuildError(#[from] reqwest::Error);

/// sysand HTTP user agent
pub(crate) const USER_AGENT: &str = concat!("sysand/", env!("CARGO_PKG_VERSION"));

/// Timeouts and retries of the client made by [`create_reqwest_client_with`].
///
/// There is no limit on the duration of a whole request, so a large download
/// on a slow but working connection always finishes.
///
/// Git sources are fetched by gix with its own HTTP client, to which these
/// options do not apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpOptions {
    /// How long to wait for a connection to be established. The read
    /// timeout is used instead when it is shorter.
    pub connect_timeout: Duration,
    /// How long to wait for each read of a response. Every successful read
    /// starts the wait anew.
    pub read_timeout: Duration,
    /// How many times an idempotent request is retried after a connection
    /// error, a timeout, or a `408`, `429` or `5xx` response.
    pub retries: u32,
    /// Hosts whose TLS certificates are not verified at all. A redirect to
    /// one of them is followed without verification, and one to any other
    /// host with it. Has no effect on plain `http://` URLs.
    pub insecure_hosts: Vec<InsecureHost>,
}

impl HttpOptions {
    pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
    pub const DEFAULT_RETRIES: u32 = 3;

    /// Set the connect and read timeouts of `builder`.
    pub(crate) fn apply_timeouts(&self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        builder
            .connect_timeout(self.connect_timeout.min(self.read_timeout))
            .read_timeout(self.read_timeout)
    }
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Self::DEFAULT_CONNECT_TIMEOUT,
            read_timeout: Self::DEFAULT_READ_TIMEOUT,
            retries: Self::DEFAULT_RETRIES,
            insecure_hosts: Vec::new(),
        }
    }
}

/// Create a reqwest client with the default [`HttpOptions`].
/// See [`create_reqwest_client_with`].
pub fn create_reqwest_client()
-> Result<reqwest_middleware::ClientWithMiddleware, ReqwestClientBuildError> {
    create_reqwest_client_with(&HttpOptions::default())
}

/// Create a reqwest client. This must be used where possible.
/// Note that gix manages its own HTTP client, so logs may indicate
/// duplicate initialization.
///
/// Follows up to 10 redirects, as `reqwest` does by default; index clients
/// MUST follow HTTP redirects on the index URL discovery fetch and on every
/// index resource. This holds for [`HttpOptions::insecure_hosts`] too.
pub fn create_reqwest_client_with(
    options: &HttpOptions,
) -> Result<reqwest_middleware::ClientWithMiddleware, ReqwestClientBuildError> {
    install_default_crypto_provider();

    let mut client = options.apply_timeouts(client_builder());
    if !options.insecure_hosts.is_empty() {
        // `InsecureHostsRouter` follows every redirect, to choose the client
        // for each hop
        client = client.redirect(reqwest::redirect::Policy::none());
    }
    let client = client.build()?;

    let mut builder = reqwest_middleware::ClientBuilder::new(client);
    if options.retries > 0 {
        builder = builder.with(Retry {
            retries: options.retries,
        });
    }
    if !options.insecure_hosts.is_empty() {
        log::warn!(
            "TLS certificate verification is DISABLED for: {}",
            options
                .insecure_hosts
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
        // After `Retry`, so that requests to these hosts are retried too.
        // A retry repeats the whole redirect chain from the original URL, not
        // only the failed hop, as `Retry` wraps all redirect following.
        // Must be the last middleware: it sends these requests through its
        // own client instead of `next`, so any middleware after it would be
        // skipped for them.
        builder = builder.with(InsecureHostsRouter::new(options)?);
    }
    Ok(builder.build())
}

/// A host whose TLS certificate is not verified, see
/// [`HttpOptions::insecure_hosts`]: a domain name or IP address only, with no
/// scheme, port or path. Parse one from a string, with IPv6 addresses in
/// brackets as in a URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InsecureHost {
    /// Lowercase
    Domain(String),
    Ip(IpAddr),
}

impl std::str::FromStr for InsecureHost {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match url::Host::parse(value) {
            Ok(url::Host::Domain(domain)) if !domain.is_empty() => Ok(Self::Domain(domain)),
            Ok(url::Host::Ipv4(ip)) => Ok(Self::Ip(ip.into())),
            Ok(url::Host::Ipv6(ip)) => Ok(Self::Ip(ip.into())),
            _ => Err(format!(
                "`{value}` is not a host name or IP address only (no scheme, port or path; \
                 IPv6 addresses go in brackets)"
            )),
        }
    }
}

impl Display for InsecureHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Domain(domain) => f.write_str(domain),
            Self::Ip(ip) => write!(f, "{ip}"),
        }
    }
}

impl InsecureHost {
    fn matches(&self, url: &Url) -> bool {
        match (self, url.host()) {
            (Self::Domain(domain), Some(url::Host::Domain(host))) => domain == host,
            (Self::Ip(ip), Some(url::Host::Ipv4(host))) => *ip == IpAddr::from(host),
            (Self::Ip(ip), Some(url::Host::Ipv6(host))) => *ip == IpAddr::from(host),
            _ => false,
        }
    }
}

fn is_insecure(hosts: &[InsecureHost], url: &Url) -> bool {
    hosts.iter().any(|host| host.matches(url))
}

/// Sends requests to [`HttpOptions::insecure_hosts`] through a second client
/// that does not verify TLS certificates, and all others through the next
/// client in the stack.
///
/// Neither client follows redirects, the router does, so that each hop is
/// sent through the client for its own host. Thus no unlisted host is ever
/// contacted without verification, and a redirect between a listed and an
/// unlisted host, in either direction, is followed.
///
/// A request whose body cannot be cloned, such as a streamed upload, is not
/// redirected, its redirect response is returned as is.
struct InsecureHostsRouter {
    client: reqwest::Client,
    hosts: Vec<InsecureHost>,
}

impl InsecureHostsRouter {
    /// Matches `reqwest`'s default limit
    const MAX_REDIRECTS: usize = 10;

    fn new(options: &HttpOptions) -> Result<Self, ReqwestClientBuildError> {
        let client = options
            .apply_timeouts(client_builder())
            .danger_accept_invalid_certs(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            hosts: options.insecure_hosts.clone(),
        })
    }
}

/// The request that follows the redirect `response` to `request`, or `None`
/// if `response` is not a redirect that can be followed. Adjusts the method,
/// body and headers like `reqwest` does for its own redirects.
fn redirect_request(
    mut request: reqwest::Request,
    response: &reqwest::Response,
) -> Option<reqwest::Request> {
    use reqwest::{Method, StatusCode};

    let status = response.status();
    if !matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    ) {
        return None;
    }
    let location = response.headers().get(header::LOCATION)?.to_str().ok()?;
    let target = request.url().join(location).ok()?;
    if !matches!(target.scheme(), "http" | "https") {
        return None;
    }

    let to_get = match status {
        StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND => request.method() == Method::POST,
        StatusCode::SEE_OTHER => {
            request.method() != Method::GET && request.method() != Method::HEAD
        }
        _ => false,
    };
    if to_get {
        *request.method_mut() = Method::GET;
        *request.body_mut() = None;
        for name in [
            header::TRANSFER_ENCODING,
            header::CONTENT_ENCODING,
            header::CONTENT_TYPE,
            header::CONTENT_LENGTH,
        ] {
            request.headers_mut().remove(name);
        }
    }
    // Credentials are for the origin they were meant for
    if target.origin() != request.url().origin() {
        for name in [
            header::AUTHORIZATION,
            header::COOKIE,
            header::PROXY_AUTHORIZATION,
            header::WWW_AUTHENTICATE,
        ] {
            request.headers_mut().remove(name);
        }
    }
    *request.url_mut() = target;
    Some(request)
}

#[async_trait::async_trait]
impl reqwest_middleware::Middleware for InsecureHostsRouter {
    async fn handle(
        &self,
        mut req: reqwest::Request,
        extensions: &mut http::Extensions,
        next: reqwest_middleware::Next<'_>,
    ) -> reqwest_middleware::Result<reqwest::Response> {
        for redirects in 0.. {
            let follow = req.try_clone();
            let response = if is_insecure(&self.hosts, req.url()) {
                self.client.execute(req).await?
            } else {
                next.clone().run(req, extensions).await?
            };
            let Some(redirect) = follow.and_then(|req| redirect_request(req, &response)) else {
                return Ok(response);
            };
            if redirects >= Self::MAX_REDIRECTS {
                return Err(reqwest_middleware::Error::middleware(
                    std::io::Error::other(format!(
                        "too many redirects, last one to `{}`",
                        redirect.url()
                    )),
                ));
            }
            req = redirect;
        }
        unreachable!("the loop only ends by returning")
    }
}

/// The backoff before the first retry, doubled before each further one. Unit
/// tests count retries against local mock servers and need no wait.
const RETRY_BASE_DELAY: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(500)
};

/// The longest wait before a retry, whatever the backoff or a `Retry-After`
/// header asks for.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(10);

/// Retries idempotent requests that failed in a way a later attempt may not.
///
/// A request whose body cannot be cloned, such as a streamed upload, is sent
/// once. So is a non-idempotent one, as the server may have acted on it.
/// Failures while reading a response body are not retried, since the
/// response has been handed to the caller by then.
struct Retry {
    retries: u32,
}

#[async_trait::async_trait]
impl reqwest_middleware::Middleware for Retry {
    async fn handle(
        &self,
        req: reqwest::Request,
        extensions: &mut http::Extensions,
        next: reqwest_middleware::Next<'_>,
    ) -> reqwest_middleware::Result<reqwest::Response> {
        let mut delay = RETRY_BASE_DELAY;
        let mut attempt = 0;
        loop {
            let retry_req = if attempt < self.retries && req.method().is_idempotent() {
                req.try_clone()
            } else {
                None
            };
            let Some(retry_req) = retry_req else {
                return next.run(req, extensions).await;
            };
            let result = next.clone().run(retry_req, extensions).await;
            let Some(reason) = retry_reason(&result) else {
                return result;
            };
            attempt += 1;
            let wait = result
                .as_ref()
                .ok()
                .and_then(retry_after)
                .unwrap_or_else(|| jittered(delay))
                .min(MAX_RETRY_DELAY);
            log::warn!(
                "request to `{}` failed ({reason}), retrying in {:.1}s ({attempt}/{})",
                RedactedUrl::from(req.url()),
                wait.as_secs_f64(),
                self.retries
            );
            tokio::time::sleep(wait).await;
            delay = delay.saturating_mul(2).min(MAX_RETRY_DELAY);
        }
    }
}

/// Why `result` is worth retrying, or `None` if it is not.
fn retry_reason(result: &reqwest_middleware::Result<reqwest::Response>) -> Option<String> {
    match result {
        Ok(response) => {
            let status = response.status();
            (status.is_server_error()
                || status == reqwest::StatusCode::REQUEST_TIMEOUT
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS)
                .then(|| format!("HTTP status {status}"))
        }
        Err(reqwest_middleware::Error::Reqwest(err)) => {
            // `io::Error::source()` skips the error an `io::Error` wraps, so
            // that is stepped into by hand
            let chain = std::iter::successors(Some(err as &dyn Error), |&e| {
                match e.downcast_ref::<std::io::Error>() {
                    Some(io) => io.get_ref().map(|inner| inner as &dyn Error),
                    None => e.source(),
                }
            });
            // A TLS failure, such as an untrusted certificate, is reported as
            // a connection error but fails the same way every time.
            if chain.clone().any(<dyn Error>::is::<rustls::Error>) {
                None
            } else if err.is_timeout() {
                Some("timed out".to_owned())
            } else if err.is_connect() || err.is_request() {
                // DNS failures are retried too: the system resolver reports
                // an unknown host and a temporary failure alike, as text
                // only, so the two cannot be told apart.
                //
                // The innermost error is the specific one, such as
                // "Connection refused".
                chain.last().map(ToString::to_string)
            } else {
                None
            }
        }
        Err(reqwest_middleware::Error::Middleware(_)) => None,
    }
}

/// `delay` plus a random part of it, so that clients failing together do
/// not all retry together.
fn jittered(delay: Duration) -> Duration {
    // `RandomState` is seeded randomly, which is all the randomness needed,
    // and spares a dependency such as `getrandom` for one random number
    #[expect(
        clippy::cast_possible_truncation,
        reason = "any 32 bits of a random number are random"
    )]
    let random = std::hash::RandomState::new().hash_one(()) as u32;
    delay + delay.saturating_mul(random) / u32::MAX
}

/// The wait a `429` or `503` response asks for in its `Retry-After` header.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    if !matches!(
        response.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::SERVICE_UNAVAILABLE
    ) {
        return None;
    }
    let value = response.headers().get(header::RETRY_AFTER)?.to_str().ok()?;
    parse_retry_after(value, jiff::Timestamp::now())
}

/// Parse a `Retry-After` value, either seconds or an HTTP date. A date
/// that is not in the future is `None`.
fn parse_retry_after(value: &str, now: jiff::Timestamp) -> Option<Duration> {
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = jiff::fmt::rfc2822::parse(value).ok()?.timestamp();
    Duration::try_from(now.duration_until(at)).ok()
}

/// A client builder with Sysand's user agent, the base of every client.
///
/// Unit tests talk only to local mock servers, so their clients never use
/// a proxy, whether from the environment or the system settings.
pub(crate) fn client_builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder().user_agent(USER_AGENT);
    #[cfg(test)]
    let builder = builder.no_proxy();
    builder
}

/// Install the crypto provider selected by this crate's `tls-*` features as
/// the process-wide rustls default.
///
/// `reqwest` is taken with `rustls-no-provider` so that no consumer is forced
/// onto a particular provider. Every path that can reach a TLS handshake
/// must therefore call this first.
///
/// Only the first call installs the provider, subsequent ones are a no-op.
///
/// With neither `tls-aws-lc-rs` nor `tls-ring` enabled this is a no-op, and
/// installing a provider is the application's job.
pub fn install_default_crypto_provider() {
    #[cfg(feature = "tls-aws-lc-rs")]
    {
        _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
    // `not(tls-aws-lc-rs)` because the two features are additive and
    // `--all-features` enables both; aws-lc-rs wins as the shipped default.
    #[cfg(all(feature = "tls-ring", not(feature = "tls-aws-lc-rs")))]
    {
        _ = rustls::crypto::ring::default_provider().install_default();
    }
    #[cfg(not(any(feature = "tls-aws-lc-rs", feature = "tls-ring")))]
    {
        log::debug!(
            "sysand-core was built with no `tls-*` feature; the application \
             must install a rustls crypto provider itself, or building an \
             HTTPS client will fail"
        );
    }
}

// Both tests assert that a provider is installed, so they only make sense
// when a `tls-*` feature selected one. Gating on `networking` alone would
// assert it in `--features networking`, which deliberately has none.
#[cfg(all(test, any(feature = "tls-aws-lc-rs", feature = "tls-ring")))]
#[path = "./net_utils_tests.rs"]
mod tests;
