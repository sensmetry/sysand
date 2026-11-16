// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! This module includes utilities for creating and using authentication policies for requests.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use globset::{Glob, GlobBuilder, GlobSetBuilder};
use jiff::Timestamp;
use reqwest::{Response, StatusCode, header};
use reqwest_middleware::{ClientWithMiddleware, RequestBuilder};
use url::{Position, Url};

use crate::credential_store::{
    CredentialRecord, CredentialScheme, CredentialStoreError,
    keyring_store::{BlobBackend, LockedBlobStore},
};
use crate::redact::RedactedUrl;

pub trait HTTPAuthentication: std::fmt::Debug + 'static {
    /// Tries to execute a request with some authentication policy. The request might be retried
    /// multiple times and it may generate auxiliary requests (using the provided client).
    fn with_authentication<F>(
        &self,
        client: &ClientWithMiddleware,
        renew_request: &F,
    ) -> impl Future<Output = Result<Response, reqwest_middleware::Error>>
    where
        F: Fn(&ClientWithMiddleware) -> RequestBuilder + 'static,
    {
        async {
            self.request_with_authentication(renew_request(client), renew_request)
                .await
        }
    }

    fn request_with_authentication<F>(
        &self,
        request: RequestBuilder,
        renew_request: &F,
    ) -> impl Future<Output = Result<Response, reqwest_middleware::Error>>
    where
        F: Fn(&ClientWithMiddleware) -> RequestBuilder + 'static;
}

/// Authentication policy that does no authentication
#[derive(Debug, Clone, Copy)]
pub struct Unauthenticated;

impl HTTPAuthentication for Unauthenticated {
    async fn request_with_authentication<F>(
        &self,
        request: RequestBuilder,
        _renew_request: &F,
    ) -> Result<Response, reqwest_middleware::Error>
    where
        F: Fn(&ClientWithMiddleware) -> RequestBuilder + 'static,
    {
        let (client, req) = request.build_split();
        let req = req?;
        log::debug!(
            "{} (no auth) `{}`",
            req.method(),
            RedactedUrl::from(req.url())
        );

        let resp = client.execute(req).await?;
        // useful to log final URL in case redirects happen
        log::debug!(
            "response to (no auth) `{}`: status {}, content type {:?}",
            resp.url(),
            resp.status(),
            resp.headers().get(header::CONTENT_TYPE)
        );
        Ok(resp)
    }
}

/// Authentication policy that *always* sends a username/password pair
#[derive(Clone)]
pub struct ForceHTTPBasicAuth {
    pub username: Box<str>,
    pub password: Box<str>,
}

// Hand-written so the password is never rendered; secret-bearing leaves
// must redact in `Debug` (see `ForceBearerAuth`).
impl std::fmt::Debug for ForceHTTPBasicAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForceHTTPBasicAuth")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl HTTPAuthentication for ForceHTTPBasicAuth {
    async fn request_with_authentication<F>(
        &self,
        request: RequestBuilder,
        _renew_request: &F,
    ) -> Result<Response, reqwest_middleware::Error>
    where
        F: Fn(&ClientWithMiddleware) -> RequestBuilder + 'static,
    {
        let (client, req) = request
            .basic_auth(&self.username, Some(&self.password))
            .build_split();
        let req = req?;
        log::debug!(
            "{} (basic auth) `{}`",
            req.method(),
            RedactedUrl::from(req.url())
        );

        let resp = client.execute(req).await?;
        log::debug!(
            "response to (basic auth) `{}`: status {}, content type {:?}",
            resp.url(),
            resp.status(),
            resp.headers().get(header::CONTENT_TYPE)
        );
        Ok(resp)
    }
}

/// Authentication policy that *always* includes a bearer token
#[derive(Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct ForceBearerAuth(Box<str>);

// Hand-written so the token is never rendered: redacting the leaf keeps
// every wrapper's `Debug` from leaking it.
impl std::fmt::Debug for ForceBearerAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ForceBearerAuth")
            .field(&"<redacted>")
            .finish()
    }
}

impl ForceBearerAuth {
    pub fn new(token: impl AsRef<str>) -> Self {
        Self(token.as_ref().into())
    }

    /// The raw token, for the `auth whoami` probe. Crate-private so the
    /// secret stays out of the public API; gated like `commands::auth`,
    /// its only consumer.
    #[cfg(all(feature = "filesystem", feature = "networking"))]
    pub(crate) fn token(&self) -> &str {
        &self.0
    }
}

impl HTTPAuthentication for ForceBearerAuth {
    async fn request_with_authentication<F>(
        &self,
        request: RequestBuilder,
        _renew_request: &F,
    ) -> Result<Response, reqwest_middleware::Error>
    where
        F: Fn(&ClientWithMiddleware) -> RequestBuilder + 'static,
    {
        let (client, req) = request.bearer_auth(&self.0).build_split();
        let req = req?;
        log::debug!(
            "{} (bearer auth) `{}`",
            req.method(),
            RedactedUrl::from(req.url())
        );

        let resp = client.execute(req).await?;
        log::debug!(
            "response to (bearer auth) `{}`: status {}, content type {:?}",
            resp.url(),
            resp.status(),
            resp.headers().get(header::CONTENT_TYPE)
        );
        Ok(resp)
    }
}

/// First tries `Higher` priority authentication and then the
/// `Lower` priority one in case the first request results in
/// a response in the 4xx range.
#[derive(Debug, Clone)]
pub struct SequenceAuthentication<Higher, Lower> {
    higher: Higher,
    lower: Lower,
}

impl<Higher: HTTPAuthentication, Lower: HTTPAuthentication> HTTPAuthentication
    for SequenceAuthentication<Higher, Lower>
{
    async fn request_with_authentication<F>(
        &self,
        request: RequestBuilder,
        renew_request: &F,
    ) -> Result<Response, reqwest_middleware::Error>
    where
        F: Fn(&ClientWithMiddleware) -> RequestBuilder + 'static,
    {
        let (client, current_request_result) = request.build_split();
        let current_request = current_request_result?;

        let initial_response = self
            .higher
            .request_with_authentication(
                RequestBuilder::from_parts(client.clone(), current_request),
                renew_request,
            )
            .await?;

        // Many servers (e.g. GitLab pages) generate a 404 instead of a 401 or 403 in response
        // to lack of authentication. A 429 is exempt: rate limiting is not
        // an auth verdict, and retrying with other credentials would spend
        // more of the rate budget on a host that just throttled us.
        let status = initial_response.status();
        if status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS {
            log::debug!("higher priority auth request returned status {status}, trying lower");
            self.lower
                .request_with_authentication(renew_request(&client), renew_request)
                .await
        } else {
            Ok(initial_response)
        }
    }
}

/// An invalid credential URL glob.
#[derive(Debug, thiserror::Error)]
pub enum UrlGlobError {
    #[error("credential URL glob `{0}` must start with `http://` or `https://` and a host")]
    NoHttpOrigin(String),
    #[error(
        "credential URL glob `{0}` has no path, so it matches no URL;\n\
         add `/**` to cover the whole host"
    )]
    NoPath(String),
    #[error("credential URL glob `{pattern}` starts with an invalid URL origin")]
    InvalidOrigin {
        pattern: String,
        #[source]
        source: Option<url::ParseError>,
    },
    #[error("credential URL glob `{0}` has a `.` or `..` path segment")]
    DotSegment(String),
    #[error("credential URL glob `{0}` has a `#` fragment")]
    Fragment(String),
    #[error(
        "credential URL glob `{0}` contains `%3F`, which cannot be told apart\n\
         from the `?` wildcard"
    )]
    EncodedQuestionMark(String),
    #[error("URL `{0}` contains `*` or `?`, which a credential URL glob cannot match literally")]
    LiteralWildcard(String),
    #[error("invalid credential URL glob `{pattern}`: {kind}")]
    Glob {
        pattern: String,
        kind: globset::ErrorKind,
    },
    #[error(transparent)]
    GlobSet(#[from] globset::Error),
}

/// Compile a credential URL glob, matched against `url::Url::as_str()`.
///
/// The wildcards are `*` and `?`, matching within one path segment, and
/// `**`; every other character is literal. The scheme is `http` or `https`,
/// and `?` may appear only in the port and path. The pattern is normalized
/// as a request URL is, so `https://Example.com:443/a b/**` matches
/// `https://example.com/a%20b/x`. A port with wildcards that can match the
/// scheme's default port also matches a URL without a port.
pub fn compile_url_glob(pattern: &str) -> Result<Glob, UrlGlobError> {
    let Some(UrlGlobParts {
        scheme,
        host,
        port,
        path,
    }) = split_url_glob(pattern)
    else {
        return Err(UrlGlobError::NoHttpOrigin(pattern.to_owned()));
    };
    if path.is_empty() {
        return Err(UrlGlobError::NoPath(pattern.to_owned()));
    }
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return Err(UrlGlobError::NoHttpOrigin(pattern.to_owned()));
    }
    if path.to_ascii_lowercase().contains("%3f") {
        return Err(UrlGlobError::EncodedQuestionMark(pattern.to_owned()));
    }
    if path.split(['/', '\\']).any(is_dot_segment) {
        return Err(UrlGlobError::DotSegment(pattern.to_owned()));
    }
    let invalid_origin = |source| UrlGlobError::InvalidOrigin {
        pattern: pattern.to_owned(),
        source,
    };
    // A URL parser skips extra slashes after an empty host, which would
    // turn the path into the host, and reads a `:` left in a host other
    // than an IPv6 address as the start of a port.
    if host.is_empty() || host.contains('?') || (host.contains(':') && !host.starts_with('[')) {
        return Err(invalid_origin(None));
    }
    let port_glob = port.filter(|port| port.contains(['*', '?']));
    if port_glob.is_some_and(|port| {
        !port
            .chars()
            .all(|c| matches!(c, '*' | '?') || c.is_ascii_digit())
    }) {
        return Err(invalid_origin(None));
    }
    // A port without wildcards is normalized by the URL parser.
    let literal_port = match port {
        Some(port) if port_glob.is_none() => format!(":{port}"),
        _ => String::new(),
    };

    // `?` would start a query, so it passes through the parser as `%3F`.
    let url = Url::parse(&format!(
        "{scheme}://{host}{literal_port}{}",
        path.replace('?', "%3F")
    ))
    .map_err(|err| invalid_origin(Some(err)))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid_origin(None));
    }
    if url.fragment().is_some() {
        return Err(UrlGlobError::Fragment(pattern.to_owned()));
    }
    let origin = escape_brackets(&url[..Position::BeforePath]);
    let path = escape_brackets(&url[Position::BeforePath..]).replace("%3F", "?");
    let glob = match port_glob {
        None => format!("{origin}{path}"),
        Some(port) => {
            let default_port = if scheme == "https" { "443" } else { "80" };
            if Glob::new(port).is_ok_and(|glob| glob.compile_matcher().is_match(default_port)) {
                // A URL with the default port has none. globset drops an
                // empty alternative, so both alternatives end in the
                // path's leading `/`.
                let after_slash = path.trim_prefix('/');
                format!("{origin}{{/,:{port}/}}{after_slash}")
            } else {
                format!("{origin}:{port}{path}")
            }
        }
    };
    GlobBuilder::new(&glob)
        .literal_separator(true)
        .build()
        .map_err(|err| UrlGlobError::Glob {
            pattern: pattern.to_owned(),
            kind: err.kind().clone(),
        })
}

/// The credential URL glob matching every URL under `root`, an absolute URL
/// ending in `/`: `root` followed by `**`. A `root` containing `*` or `?`
/// is an error, as these would act as wildcards.
pub fn url_glob_under(root: &str) -> Result<String, UrlGlobError> {
    if root.contains(['*', '?']) {
        return Err(UrlGlobError::LiteralWildcard(root.to_owned()));
    }
    let glob = format!("{root}**");
    compile_url_glob(&glob)?;
    Ok(glob)
}

#[derive(Debug, Clone)]
pub struct GlobMapBuilder<T> {
    keys: Vec<String>,
    globset: GlobSetBuilder,
    values: Vec<T>,
}

#[derive(Debug, Clone)]
pub struct GlobMap<T> {
    keys: Vec<String>,
    values: Vec<T>,
    globset: globset::GlobSet,
}

impl<T> Default for GlobMap<T> {
    /// An empty map: every lookup is `NotFound`. Unlike `GlobMapBuilder::build`,
    /// constructing the empty map cannot fail.
    fn default() -> Self {
        Self {
            keys: vec![],
            values: vec![],
            globset: globset::GlobSet::empty(),
        }
    }
}

impl<T> Default for GlobMapBuilder<T> {
    fn default() -> Self {
        Self {
            keys: vec![],
            globset: GlobSetBuilder::new(),
            values: vec![],
        }
    }
}

impl<T> GlobMapBuilder<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compile the credential URL glob `globstr` and map it to `value`.
    pub fn add<S: AsRef<str>>(&mut self, globstr: S, value: T) -> Result<(), UrlGlobError> {
        let globstr = globstr.as_ref();
        self.globset.add(compile_url_glob(globstr)?);
        self.keys.push(globstr.to_owned());
        self.values.push(value);
        Ok(())
    }

    pub fn build(self) -> Result<GlobMap<T>, globset::Error> {
        Ok(GlobMap {
            keys: self.keys,
            values: self.values,
            globset: self.globset.build()?,
        })
    }
}

#[derive(Debug)]
pub enum GlobMapResult<'a, T> {
    /// A unique matching pattern
    Found(&'a T),
    /// No matching pattern
    NotFound,
    /// Multiple matching patterns
    Ambiguous(Vec<(String, &'a T)>),
}

#[derive(Debug)]
pub enum GlobMapResultMut<'a, T> {
    /// A unique matching pattern
    Found(&'a mut T),
    /// No matching pattern
    NotFound,
    /// Multiple matching patterns
    Ambiguous(Vec<(String, &'a mut T)>),
}

impl<T> GlobMap<T> {
    pub fn lookup<'a>(&'a self, key: &str) -> GlobMapResult<'a, T> {
        let outcome = self.globset.matches(key);
        if outcome.is_empty() {
            GlobMapResult::NotFound
        } else if outcome.len() == 1 {
            GlobMapResult::Found(&self.values[outcome[0]])
        } else {
            // Indices are ascending; walk one iterator forward by gaps
            // (`nth` consumes) to match `lookup_mut`, which cannot index
            // a mutably borrowed vec repeatedly.
            let mut result = Vec::with_capacity(outcome.len());
            let mut values_iter = self.values.iter();

            let mut base = 0;
            for idx in outcome {
                result.push((self.keys[idx].clone(), values_iter.nth(idx - base).unwrap()));
                base = idx + 1;
            }

            GlobMapResult::Ambiguous(result)
        }
    }

    pub fn lookup_mut<'a>(&'a mut self, key: &str) -> GlobMapResultMut<'a, T> {
        let outcome = self.globset.matches(key);
        if outcome.is_empty() {
            GlobMapResultMut::NotFound
        } else if outcome.len() == 1 {
            GlobMapResultMut::Found(&mut self.values[outcome[0]])
        } else {
            // Same gap-walk as `lookup`, forced here by the mutable borrow.
            let mut result = Vec::with_capacity(outcome.len());
            let mut mut_values_iter = self.values.iter_mut();

            let mut base = 0;
            for idx in outcome {
                result.push((
                    self.keys[idx].clone(),
                    mut_values_iter.nth(idx - base).unwrap(),
                ));
                base = idx + 1;
            }

            GlobMapResultMut::Ambiguous(result)
        }
    }
}

/// What credential selection found in one source's bearer map for a URL.
/// Shared by the runtime read retry, whoami, and publish so the collapse
/// and ambiguity semantics are written once.
#[derive(Debug)]
pub(crate) enum BearerSelection<'a, T> {
    /// No matching pattern: the caller falls through to its next source.
    None,
    /// Exactly one credential in effect: either a unique match, or
    /// several matching patterns all carrying the identical token,
    /// collapsed to the first so any later hint names that entry.
    Unique(&'a T),
    /// Matching patterns carry distinct tokens. `matched` is the full
    /// pre-collapse match list in map order (what ambiguity errors name),
    /// `deduped` the distinct-token entries in map order (what try-all
    /// consumers walk); it always holds at least two entries.
    Ambiguous {
        // Read only by whoami and publish; without `filesystem` those
        // consumers are compiled out and only `deduped` is used.
        #[allow(
            dead_code,
            clippy::allow_attributes,
            reason = "difficult to satisfy all feature combinations with `expect`"
        )]
        matched: Vec<&'a T>,
        deduped: Vec<&'a T>,
    },
}

/// Select the bearer credential(s) for `url` from one source's map.
///
/// Several matching patterns are a real ambiguity only between *distinct*
/// tokens (`token` extracts them to compare). Handling stays with the
/// caller: publish and whoami refuse ambiguity, the runtime read path
/// tries each candidate.
pub(crate) fn select_bearer<'a, T>(
    map: &'a GlobMap<T>,
    url: &str,
    token: impl Fn(&T) -> &str,
) -> BearerSelection<'a, T> {
    match map.lookup(url) {
        GlobMapResult::NotFound => BearerSelection::None,
        GlobMapResult::Found(entry) => BearerSelection::Unique(entry),
        GlobMapResult::Ambiguous(candidates) => {
            let mut deduped: Vec<&T> = Vec::new();
            for (_, entry) in &candidates {
                if !deduped.iter().any(|seen| token(seen) == token(entry)) {
                    deduped.push(entry);
                }
            }
            if deduped.len() == 1 {
                BearerSelection::Unique(deduped[0])
            } else {
                BearerSelection::Ambiguous {
                    matched: candidates.into_iter().map(|(_, entry)| entry).collect(),
                    deduped,
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
/// Uses `restricted` authentication only on urls matching one of specified globs,
/// otherwise use `unrestricted`. For an ambiguous match a warning is generated and the
/// ambiguous options are tried, in order, until a non-4xx response is generated. If no
/// option produces a non-4xx response, the *first* response is returned.
///
/// Note that redirects work differently (due to reqwest internal defaults):
/// - if `restricted` is used for a URL, it is also used for the redirect target if
///   the target is on the same host
/// - `unrestricted` is not tried for redirect target URL.
pub struct RestrictAuthentication<Restricted, Unrestricted> {
    pub restricted: GlobMap<Restricted>,
    pub unrestricted: Unrestricted,
}

impl<Restricted: HTTPAuthentication, Unrestricted: HTTPAuthentication> HTTPAuthentication
    for RestrictAuthentication<Restricted, Unrestricted>
{
    async fn request_with_authentication<F>(
        &self,
        request: RequestBuilder,
        renew_request: &F,
    ) -> Result<Response, reqwest_middleware::Error>
    where
        F: Fn(&ClientWithMiddleware) -> RequestBuilder + 'static,
    {
        let (client, current_request_result) = request.build_split();
        let current_request = current_request_result?;

        let url = current_request.url();
        match self.restricted.lookup(url.as_str()) {
            GlobMapResult::Found(restricted) => {
                restricted
                    .request_with_authentication(
                        RequestBuilder::from_parts(client.clone(), current_request),
                        renew_request,
                    )
                    .await
            }
            GlobMapResult::NotFound => {
                self.unrestricted
                    .request_with_authentication(
                        RequestBuilder::from_parts(client.clone(), current_request),
                        renew_request,
                    )
                    .await
            }
            GlobMapResult::Ambiguous(items) => {
                let items: Vec<_> = items.into_iter().collect();

                let matched_patterns = items
                    .iter()
                    .fold(String::new(), |acc, (p, _)| acc + "\n" + p);
                log::warn!(
                    "URL {} matches multiple authentication URL globs: {}",
                    url.as_str(),
                    matched_patterns
                );

                let mut items = items.into_iter();
                let (_, first_restricted) = items.next().unwrap();
                let first_response = first_restricted
                    .request_with_authentication(
                        RequestBuilder::from_parts(client.clone(), current_request),
                        renew_request,
                    )
                    .await?;
                if first_response.status().is_client_error() {
                    for (_, other_restricted) in items {
                        let other_response = other_restricted
                            .with_authentication(&client, renew_request)
                            .await?;
                        if !other_response.status().is_client_error() {
                            return Ok(other_response);
                        }
                    }
                }
                Ok(first_response)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum StandardInnerAuthentication {
    HTTPBasicAuth(ForceHTTPBasicAuth),
    BearerAuth {
        auth: ForceBearerAuth,
        /// The `SYSAND_CRED_<LABEL>` stem this bearer was built from.
        /// Display-only: publish auth failures name the variable to fix.
        env_label: Box<str>,
    },
}

impl HTTPAuthentication for StandardInnerAuthentication {
    async fn request_with_authentication<F>(
        &self,
        request: RequestBuilder,
        renew_request: &F,
    ) -> Result<Response, reqwest_middleware::Error>
    where
        F: Fn(&ClientWithMiddleware) -> RequestBuilder + 'static,
    {
        match self {
            Self::HTTPBasicAuth(inner) => {
                inner
                    .request_with_authentication(request, renew_request)
                    .await
            }
            Self::BearerAuth { auth, env_label: _ } => {
                auth.request_with_authentication(request, renew_request)
                    .await
            }
        }
    }
}

/// Standard HTTP authentication policy where a restricted set of domains/paths have
/// Basic Auth username/password pairs specified, but they are sent only in response to a
/// 4xx status code.
pub type StandardHTTPAuthentication = RestrictAuthentication<
    SequenceAuthentication<
        // First try unauthenticated access...
        Unauthenticated,
        // ... but send username/password in response to 4xx.
        // FIXME: Replace by a more general type as more authentication schemes are added
        StandardInnerAuthentication,
    >,
    // For all other domains use unauthenticated access.
    Unauthenticated,
>;

/// One environment-sourced bearer credential as extracted for publish:
/// the token plus the `SYSAND_CRED_<LABEL>` stem it came from, so an
/// upload auth failure can name the variable to fix.
#[derive(Clone)]
pub struct EnvBearerAuth {
    pub auth: ForceBearerAuth,
    /// The `SYSAND_CRED_<LABEL>` stem.
    pub label: String,
}

impl std::fmt::Debug for EnvBearerAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Shows only the non-secret label; the token is redacted by
        // `ForceBearerAuth`'s `Debug`.
        f.debug_struct("EnvBearerAuth")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl StandardHTTPAuthentication {
    /// Extracts the bearer tokens from the configured credential set into a URL-glob map
    /// suitable for driving publish-time credential selection. Basic-auth entries are
    /// dropped, since publish only supports bearer authentication.
    pub fn publish_bearer_auth_map(&self) -> Result<GlobMap<EnvBearerAuth>, UrlGlobError> {
        let mut partial = GlobMapBuilder::new();
        let restricted = &self.restricted;
        for (key, sequence_auth) in restricted.keys.iter().zip(&restricted.values) {
            if let StandardInnerAuthentication::BearerAuth { auth, env_label } =
                &sequence_auth.lower
            {
                partial.add(
                    key,
                    EnvBearerAuth {
                        auth: auth.clone(),
                        label: env_label.to_string(),
                    },
                )?;
            }
        }

        Ok(partial.build()?)
    }
}

/// Utility to simplify construction of `StandardHTTPAuthentication`
#[derive(Debug, Default, Clone)]
pub struct StandardHTTPAuthenticationBuilder {
    partial: GlobMapBuilder<SequenceAuthentication<Unauthenticated, StandardInnerAuthentication>>,
}

impl StandardHTTPAuthenticationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(self) -> Result<StandardHTTPAuthentication, globset::Error> {
        Ok(StandardHTTPAuthentication {
            restricted: self.partial.build()?,
            unrestricted: Unauthenticated {},
        })
    }

    pub fn add_basic_auth<S: AsRef<str>, T: AsRef<str>, R: AsRef<str>>(
        &mut self,
        globstr: S,
        username: T,
        password: R,
    ) -> Result<(), UrlGlobError> {
        self.partial.add(
            globstr,
            SequenceAuthentication {
                higher: Unauthenticated {},
                lower: StandardInnerAuthentication::HTTPBasicAuth(ForceHTTPBasicAuth {
                    username: username.as_ref().into(),
                    password: password.as_ref().into(),
                }),
            },
        )
    }

    /// Add a bearer credential. `env_label` is the `SYSAND_CRED_<LABEL>`
    /// stem the credential came from; every env credential has one, and
    /// publish auth failures name the variable to fix through it.
    pub fn add_bearer_auth<S: AsRef<str>, T: AsRef<str>, L: AsRef<str>>(
        &mut self,
        globstr: S,
        token: T,
        env_label: L,
    ) -> Result<(), UrlGlobError> {
        self.partial.add(
            globstr,
            SequenceAuthentication {
                higher: Unauthenticated {},
                lower: StandardInnerAuthentication::BearerAuth {
                    auth: ForceBearerAuth::new(token),
                    env_label: env_label.as_ref().into(),
                },
            },
        )
    }
}

/// One stored credential as loaded into the lazy credential map: the
/// bearer token plus the non-secret fields runtime messages need (the key
/// for hints, `expires_at` for the expiry hint and publish's fail-fast
/// check).
#[derive(Clone)]
pub struct StoredBearerAuth {
    auth: ForceBearerAuth,
    key: String,
    expires_at: Option<Timestamp>,
    /// Whether the reactive expiry hint has already been emitted for this
    /// record. `Arc`d so the flag is shared across the record's globs and
    /// across map clones: together with the once-per-process map cache
    /// this gives at most one warning per record per process.
    /// (credential expiry is checked for every request that escalates to it,
    /// so it can happen multiple times for a single credential)
    expiry_warned: Arc<AtomicBool>,
}

impl std::fmt::Debug for StoredBearerAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Shows only the non-secret record fields; the token is redacted
        // by `ForceBearerAuth`'s `Debug`.
        f.debug_struct("StoredBearerAuth")
            .field("key", &self.key)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl StoredBearerAuth {
    pub(crate) fn new(auth: ForceBearerAuth, key: String, expires_at: Option<Timestamp>) -> Self {
        Self {
            auth,
            key,
            expires_at,
            expiry_warned: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The forced-retry bearer policy for this login.
    pub fn auth(&self) -> &ForceBearerAuth {
        &self.auth
    }

    /// The normalized index key the login was stored under, in the exact
    /// form `sysand auth login <key>` accepts.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Expiry, when a validating login learned it.
    pub fn expires_at(&self) -> Option<Timestamp> {
        self.expires_at
    }

    /// The reactive expiry hint, returned at most once per record: `Some` exactly when the record
    /// carries a past `expires_at` and no hint was emitted before.
    fn take_expiry_warning(&self, now: Timestamp) -> Option<String> {
        let expires_at = self.expires_at?;
        if expires_at >= now || self.expiry_warned.swap(true, Ordering::SeqCst) {
            return None;
        }
        Some(format!(
            "credential for `{key}` may be expired or revoked;\n\
             re-authenticate to store a fresh credential",
            key = self.key
        ))
    }

    /// Emit the reactive expiry hint after a forced retry that still
    /// ended in a 4xx. Any 4xx counts, not just 401: GitLab-style hosts
    /// answer 404 on bad auth.
    fn warn_if_expired(&self) {
        if let Some(message) = self.take_expiry_warning(Timestamp::now()) {
            log::warn!("{message}");
        }
    }

    #[cfg(test)]
    fn expiry_warning_emitted(&self) -> bool {
        self.expiry_warned.load(Ordering::SeqCst)
    }
}

/// Authentication layer that consults the persistent credential store
/// ([`LockedBlobStore`]) on demand.
///
/// `inner` (the eager `SYSAND_CRED_*` env policy) runs first; only a 4xx
/// triggers a store read (cached, at most once per process). Not a stock
/// [`SequenceAuthentication`]: with no matching record the inner response
/// is returned with no extra request, whereas a sequence's lower arm would
/// re-issue on every ordinary 404, doubling round-trips for logged-in
/// users. A matching record gets a *forced* [`ForceBearerAuth`] retry (v1
/// stores bearer only). Any 4xx escalates (GitLab-style hosts answer 404
/// on bad auth) except 429: rate limiting is not an auth verdict.
///
/// The store lookup uses the initial request URL; as with
/// [`RestrictAuthentication`], redirect targets are not re-checked.
pub struct CredentialStoreAuthentication<Inner, B> {
    inner: Inner,
    /// `None` when no store is available (for example, opening the default
    /// OS keyring store failed); behaves as an always-empty store.
    store: Option<Arc<LockedBlobStore<B>>>,
    /// Stored bearer credentials, read lazily. A `tokio` `OnceCell` (not a
    /// sync `OnceLock`) so concurrent first requests (the resolve path
    /// fans out with `join_all`) share a single read: at most one keychain
    /// touch per process.
    cache: tokio::sync::OnceCell<GlobMap<StoredBearerAuth>>,
}

/// Standard composed policy: eager `SYSAND_CRED_*` credentials first, then
/// lazily read stored credentials from a [`LockedBlobStore`] over `B`.
pub type StandardLazyHTTPAuthentication<B> =
    CredentialStoreAuthentication<StandardHTTPAuthentication, B>;

impl<Inner, B> std::fmt::Debug for CredentialStoreAuthentication<Inner, B>
where
    Inner: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Shows `has_store`/`cache_read` rather than the cache contents;
        // stored tokens are redacted by `ForceBearerAuth`'s `Debug`.
        f.debug_struct("CredentialStoreAuthentication")
            .field("inner", &self.inner)
            .field("has_store", &self.store.is_some())
            .field("cache_read", &self.cache.get().is_some())
            .finish()
    }
}

impl<Inner, B> CredentialStoreAuthentication<Inner, B>
where
    B: BlobBackend + Send + Sync + 'static,
{
    pub fn new(inner: Inner, store: LockedBlobStore<B>) -> Self {
        Self {
            inner,
            store: Some(Arc::new(store)),
            cache: tokio::sync::OnceCell::new(),
        }
    }

    /// A policy with no credential store: only `inner` ever answers.
    /// For hosts where opening the store failed.
    pub fn without_store(inner: Inner) -> Self {
        Self {
            inner,
            store: None,
            cache: tokio::sync::OnceCell::new(),
        }
    }

    /// A policy over an already-read snapshot of the store's records: the
    /// cache is seeded from `records` and no store is ever touched. For
    /// hosts that read the store once and share the snapshot between this
    /// policy and their own credential selection (`auth whoami`'s
    /// single-read guarantee).
    pub fn preloaded(inner: Inner, records: &[CredentialRecord]) -> Self {
        Self {
            inner,
            store: None,
            cache: tokio::sync::OnceCell::new_with(Some(stored_bearer_map_from_records(records))),
        }
    }

    /// The eager env-credential policy this layer wraps. Publish extracts
    /// its bearer map from here without touching the store.
    pub fn env_policy(&self) -> &Inner {
        &self.inner
    }

    /// The stored bearer credentials, read and cached on first use.
    pub async fn stored_bearer_map(&self) -> &GlobMap<StoredBearerAuth> {
        self.cache
            .get_or_init(|| async {
                let Some(store) = self.store.clone() else {
                    return GlobMap::default();
                };
                // The keyring crate is synchronous and can block for
                // seconds (unlock prompt, cross-process lock), so the
                // once-per-process read runs on the blocking pool.
                match tokio::task::spawn_blocking(move || read_stored_bearer_map(&*store)).await {
                    Ok(map) => map,
                    Err(err) => {
                        log::warn!(
                            "credential store read failed: {err};\n\
                             continuing without stored credentials"
                        );
                        GlobMap::default()
                    }
                }
            })
            .await
    }

    /// One direct store read for callers outside the async runtime
    /// (publish's credential selection). Bypasses the request path's
    /// cache; errors degrade to an empty map under
    /// [`read_stored_bearer_map`]'s warning semantics.
    pub fn read_stored_bearer_map_direct(&self) -> GlobMap<StoredBearerAuth> {
        match &self.store {
            None => GlobMap::default(),
            Some(store) => read_stored_bearer_map(store.as_ref()),
        }
    }
}

/// Read all stored records and build the URL-glob to bearer map.
///
/// Store errors degrade to "no stored credentials" rather than aborting:
/// the request may still succeed via env credentials or anonymously, and
/// hard failure is reserved for the `auth` commands.
/// `BackendAbsent` is debug-only
/// (the designed env-fallback state on keyring-less hosts; `auth status`
/// and `auth login` still report it loudly), while every other variant
/// warns: a keyring exists and something is wrong, so staying quiet would
/// silently downgrade a logged-in user to unauthenticated requests.
fn read_stored_bearer_map<B: BlobBackend>(store: &LockedBlobStore<B>) -> GlobMap<StoredBearerAuth> {
    let records = match store.list() {
        Ok(records) => records,
        Err(CredentialStoreError::BackendAbsent { source }) => {
            log::debug!(
                "no OS keyring backend ({source});\n\
                 using `SYSAND_CRED_*` credentials only"
            );
            return GlobMap::default();
        }
        Err(err) => {
            log::warn!(
                "could not read stored credentials: {err};\n\
                 unlock your OS keyring, or provide credentials via `SYSAND_CRED_*` environment variables;\n\
                 continuing without stored credentials"
            );
            return GlobMap::default();
        }
    };
    stored_bearer_map_from_records(&records)
}

/// Build the URL-glob to bearer map from credential records. Also the
/// stored-source map for whoami's credential selection, which reads the
/// records once and shares the snapshot with its discovery policy.
pub(crate) fn stored_bearer_map_from_records(
    records: &[CredentialRecord],
) -> GlobMap<StoredBearerAuth> {
    let mut builder = GlobMapBuilder::new();
    for record in records {
        // Exhaustive on purpose: adding a scheme must revisit this map.
        match record.scheme {
            CredentialScheme::Bearer => {}
        }
        // One `StoredBearerAuth` per record, cloned per glob: the clones
        // share the record's expiry-warned flag.
        let auth = StoredBearerAuth::new(
            ForceBearerAuth::new(record.secret.as_str()),
            record.key.clone(),
            record.expires_at,
        );
        for glob in &record.globs {
            let added = match glob.strip_suffix("**") {
                Some(root) if root.contains(['*', '?']) => {
                    Err(UrlGlobError::LiteralWildcard(root.to_owned()))
                }
                _ => builder.add(glob, auth.clone()),
            };
            if let Err(err) = added {
                log::warn!(
                    "ignoring a stored credential URL pattern for `{}`: {}",
                    record.key,
                    crate::utils::format_err(err)
                );
            }
        }
    }
    match builder.build() {
        Ok(map) => map,
        Err(err) => {
            log::warn!(
                "could not build stored credential URL patterns, \
                 continuing without stored credentials: {}",
                crate::utils::format_err(err)
            );
            GlobMap::default()
        }
    }
}

impl<Inner, B> HTTPAuthentication for CredentialStoreAuthentication<Inner, B>
where
    Inner: HTTPAuthentication,
    B: BlobBackend + Send + Sync + 'static,
{
    async fn request_with_authentication<F>(
        &self,
        request: RequestBuilder,
        renew_request: &F,
    ) -> Result<Response, reqwest_middleware::Error>
    where
        F: Fn(&ClientWithMiddleware) -> RequestBuilder + 'static,
    {
        let (client, current_request_result) = request.build_split();
        let current_request = current_request_result?;
        let url = current_request.url().clone();

        let initial_response = self
            .inner
            .request_with_authentication(
                RequestBuilder::from_parts(client.clone(), current_request),
                renew_request,
            )
            .await?;

        // 429 is never an auth verdict: no store read, no forced retry.
        let status = initial_response.status();
        if !status.is_client_error() || status == StatusCode::TOO_MANY_REQUESTS {
            return Ok(initial_response);
        }

        let stored = self.stored_bearer_map().await;
        // Ambiguous matches try each candidate in order until one yields
        // a non-4xx response, else the first retry response is returned.
        let deduped = match select_bearer(stored, url.as_str(), |bearer| &bearer.auth.0) {
            BearerSelection::None => return Ok(initial_response),
            BearerSelection::Unique(bearer) => {
                log::debug!(
                    "stored credential matches `{}`; retrying with forced bearer auth",
                    RedactedUrl::from(url)
                );
                vec![bearer]
            }
            BearerSelection::Ambiguous {
                deduped,
                matched: _,
            } => {
                log::warn!(
                    "URL {} matches multiple stored credentials; trying each in order",
                    RedactedUrl::from(url)
                );
                deduped
            }
        };
        let mut deduped = deduped.into_iter();
        let first_bearer = deduped.next().expect("selection produced no candidates");
        let first_response = first_bearer
            .auth
            .request_with_authentication(renew_request(&client), renew_request)
            .await?;
        if !first_response.status().is_client_error() {
            return Ok(first_response);
        }
        first_bearer.warn_if_expired();
        for bearer in deduped {
            let response = bearer
                .auth
                .with_authentication(&client, renew_request)
                .await?;
            if !response.status().is_client_error() {
                return Ok(response);
            }
            bearer.warn_if_expired();
        }
        Ok(first_response)
    }
}

/// Whether the URL path segment `segment` is `.` or `..`, which a URL
/// parser resolves, also when written with `%2e`.
fn is_dot_segment(segment: &str) -> bool {
    matches!(
        segment.to_ascii_lowercase().replace("%2e", ".").as_str(),
        "." | ".."
    )
}

/// `text` with `[`, `]`, `{` and `}` escaped, so that globset reads them
/// literally.
fn escape_brackets(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '[' | ']' | '{' | '}') {
            escaped.push('[');
            escaped.push(c);
            escaped.push(']');
        } else {
            escaped.push(c);
        }
    }
    escaped
}

/// The parts of a URL glob, as written.
struct UrlGlobParts<'a> {
    scheme: &'a str,
    host: &'a str,
    port: Option<&'a str>,
    path: &'a str,
}

/// Split the URL glob `pattern` into its parts: `https://h:1/a/**` gives
/// `https`, `h`, `1` and `/a/**`. The path starts at the first `/` or `\`
/// after `://` and is empty without one. The port follows the last `:`
/// after any `]` of an IPv6 host. `None` when `pattern` has no `://`.
fn split_url_glob(pattern: &str) -> Option<UrlGlobParts<'_>> {
    let (scheme, rest) = pattern.split_once("://")?;
    let (authority, path) = rest.split_at(rest.find(['/', '\\']).unwrap_or(rest.len()));
    let port_colon = authority
        .rfind(':')
        .filter(|&colon| authority.rfind(']').is_none_or(|bracket| colon > bracket));
    let (host, port) = match port_colon {
        Some(colon) => {
            let (host, colon_and_port) = authority.split_at(colon);
            (host, colon_and_port.strip_prefix(':'))
        }
        None => (authority, None),
    };
    Some(UrlGlobParts {
        scheme,
        host,
        port,
        path,
    })
}

#[cfg(test)]
#[path = "./auth_tests.rs"]
mod tests;
