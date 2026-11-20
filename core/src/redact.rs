// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! URLs that render with credentials masked, and HTTP errors that render
//! without their URL, for error messages and logs.

use std::{borrow::Cow, fmt};

use url::{Position, Url};

/// What a masked credential is rendered as.
pub(crate) const MASK: &str = "****";

/// Query parameters that carry a signature or credential in signed URLs of
/// cloud storage and CDNs. Matched case-insensitively.
const SENSITIVE_QUERY_PARAMETERS: &[&str] = &[
    // AWS S3
    "X-Amz-Credential",
    "X-Amz-Security-Token",
    "X-Amz-Signature",
    // Azure Blob Storage, also Artifactory signed URLs
    "sig",
    // Google Cloud Storage; its V2 signed URLs use `Signature`
    "X-Goog-Credential",
    "X-Goog-Signature",
    // CloudFront
    "Key-Pair-Id",
    "Policy",
    "Signature",
];

/// A URL whose `Display` and `Debug` mask credentials: the password, a
/// username without a password (often a token), and the values of
/// [`SENSITIVE_QUERY_PARAMETERS`]. Everything else is rendered as is.
///
/// Boxed to keep the errors that hold it small.
#[derive(Clone, PartialEq, Eq)]
pub struct RedactedUrl(Box<Url>);

impl RedactedUrl {
    /// The URL with its credentials, never to be shown to the user.
    pub fn unredacted(&self) -> &Url {
        &self.0
    }
}

impl From<Url> for RedactedUrl {
    fn from(url: Url) -> Self {
        Self(Box::new(url))
    }
}

impl From<&Url> for RedactedUrl {
    fn from(url: &Url) -> Self {
        url.clone().into()
    }
}

impl fmt::Display for RedactedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let url = &*self.0;
        f.write_str(&url[..Position::BeforeUsername])?;
        if url.password().is_some() {
            write!(f, "{}:{MASK}@", url.username())?;
        } else if !url.username().is_empty() {
            write!(f, "{MASK}@")?;
        }
        f.write_str(&url[Position::BeforeHost..Position::AfterPath])?;
        if let Some(query) = url.query() {
            write!(f, "?{}", redact_query(query))?;
        }
        // The fragment with its `#`, if any
        f.write_str(&url[Position::AfterQuery..])
    }
}

impl fmt::Debug for RedactedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RedactedUrl")
            .field(&format_args!("{self}"))
            .finish()
    }
}

/// `query` with the values of [`SENSITIVE_QUERY_PARAMETERS`] masked. Other
/// parameters are kept exactly as written.
fn redact_query(query: &str) -> Cow<'_, str> {
    // The key of a `key=value` pair, if it is a sensitive one
    fn sensitive_key(pair: &str) -> Option<&str> {
        let (key, _) = pair.split_once('=')?;
        let (decoded, _) = url::form_urlencoded::parse(key.as_bytes()).next()?;
        SENSITIVE_QUERY_PARAMETERS
            .iter()
            .any(|sensitive| decoded.eq_ignore_ascii_case(sensitive))
            .then_some(key)
    }

    if !query.split('&').any(|pair| sensitive_key(pair).is_some()) {
        return Cow::Borrowed(query);
    }
    let pairs: Vec<_> = query
        .split('&')
        .map(|pair| match sensitive_key(pair) {
            Some(key) => Cow::Owned(format!("{key}={MASK}")),
            None => Cow::Borrowed(pair),
        })
        .collect();
    Cow::Owned(pairs.join("&"))
}

/// Render a possibly malformed URL for an error message with any userinfo
/// replaced by [`MASK`], so an embedded password never reaches stderr
/// or CI logs. String-based on purpose: it must work for inputs
/// `url::Url::parse` rejected.
pub(crate) fn redact_userinfo(raw: &str) -> Box<str> {
    // The authority runs from after any scheme separator to the first
    // `/`, `?`, or `#`; userinfo is everything up to the last `@` in it.
    let authority_start = raw.find("://").map_or(0, |idx| idx + 3);
    let authority_end = raw[authority_start..]
        .find(['/', '?', '#'])
        .map_or(raw.len(), |idx| authority_start + idx);
    match raw[authority_start..authority_end].rfind('@') {
        Some(at) => format!(
            "{}{MASK}@{}",
            &raw[..authority_start],
            &raw[authority_start + at + 1..]
        )
        .into(),
        None => raw.into(),
    }
}

/// The URL a request was sent to and, when redirects led elsewhere, the
/// URL the response came from, for messages about the response. Renders
/// both quoted, with credentials masked as [`RedactedUrl`] does.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResponseUrl {
    requested: RedactedUrl,
    redirected: Option<RedactedUrl>,
}

impl ResponseUrl {
    /// `response_url` is kept only if it differs from `requested`.
    pub fn new(requested: &Url, response_url: &Url) -> Self {
        Self {
            requested: requested.into(),
            redirected: (requested != response_url).then(|| response_url.into()),
        }
    }

    pub fn requested(&self) -> &RedactedUrl {
        &self.requested
    }
}

impl fmt::Display for ResponseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}`", self.requested)?;
        if let Some(redirected) = &self.redirected {
            write!(f, " (redirected to `{redirected}`)")?;
        }
        Ok(())
    }
}

/// An HTTP error rendered without the URL reqwest attaches to it: that
/// URL may hold credentials, and is the one the request was sent to even
/// when a redirect hop is what failed. The error wrapping it names the URL
/// instead, as a [`RedactedUrl`] or [`ResponseUrl`]. Otherwise rendered
/// exactly as the wrapped error.
#[cfg(feature = "networking")]
#[derive(Debug)]
pub struct HttpError(reqwest_middleware::Error);

#[cfg(feature = "networking")]
impl HttpError {
    pub fn inner(&self) -> &reqwest_middleware::Error {
        &self.0
    }
}

#[cfg(feature = "networking")]
impl From<reqwest_middleware::Error> for HttpError {
    fn from(err: reqwest_middleware::Error) -> Self {
        Self(match err {
            reqwest_middleware::Error::Reqwest(err) => {
                reqwest_middleware::Error::Reqwest(err.without_url())
            }
            middleware @ reqwest_middleware::Error::Middleware(_) => middleware,
        })
    }
}

#[cfg(feature = "networking")]
impl From<reqwest::Error> for HttpError {
    fn from(err: reqwest::Error) -> Self {
        reqwest_middleware::Error::Reqwest(err).into()
    }
}

#[cfg(feature = "networking")]
impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(feature = "networking")]
impl std::error::Error for HttpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

/// An HTTP exchange that failed: the request, a non-success status, or
/// reading the body.
#[cfg(feature = "networking")]
#[derive(thiserror::Error, Debug)]
pub enum HttpFailure {
    #[error("HTTP request to `{url}` failed")]
    Request {
        url: RedactedUrl,
        #[source]
        source: HttpError,
    },
    #[error("HTTP request to {url} returned status {status}")]
    BadHttpStatus {
        url: ResponseUrl,
        status: reqwest::StatusCode,
    },
    #[error("failed to read HTTP response body from {url}")]
    Body {
        url: ResponseUrl,
        #[source]
        source: HttpError,
    },
}

#[cfg(test)]
#[path = "./redact_tests.rs"]
mod tests;
