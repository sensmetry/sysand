// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! This module includes utilities for creating and using authentication policies for requests.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use chrono::{DateTime, Utc};
use globset::{GlobBuilder, GlobSetBuilder};
use reqwest::{Response, header};
use reqwest_middleware::{ClientWithMiddleware, RequestBuilder};

use crate::credential_store::{CredentialScheme, CredentialStore, CredentialStoreError};

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
pub struct Unauthenticated {}

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
        log::debug!("{} (no auth) `{}`", req.method(), req.url());

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

// Hand-written so the password is never rendered, matching the redaction
// on `ForceBearerAuth`. Both are secret-bearing leaves reachable through a
// composed policy's `Debug`, so both must redact.
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
        log::debug!("{} (basic auth) `{}`", req.method(), req.url());

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

// Hand-written so the token is never rendered. This is the single
// secret-bearing leaf: redacting it here keeps every wrapper's `Debug`
// (and any accidental `{:?}` on a composed policy) from leaking the token.
impl std::fmt::Debug for ForceBearerAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ForceBearerAuth")
            .field(&"<redacted>")
            .finish()
    }
}

impl ForceBearerAuth {
    pub fn new<S: AsRef<str>>(token: S) -> ForceBearerAuth {
        Self(token.as_ref().into())
    }

    /// The raw token, for callers that send the credential outside a
    /// policy chain (the `auth whoami` probe). Crate-private so the
    /// secret never leaks into the public API; gated like
    /// `commands::auth`, its only consumer.
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
        log::debug!("{} (bearer auth) `{}`", req.method(), req.url());

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
        // to lack of authentication.
        let status = initial_response.status();
        if status.is_client_error() {
            log::debug!("higher priority auth request returned status {status}, trying lower");
            self.lower
                .request_with_authentication(renew_request(&client), renew_request)
                .await
        } else {
            Ok(initial_response)
        }
    }
}

#[derive(Debug, Clone)]
pub struct GlobMapBuilder<T> {
    keys: Vec<String>,
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
        GlobMap {
            keys: vec![],
            values: vec![],
            globset: globset::GlobSet::empty(),
        }
    }
}

impl<T> Default for GlobMapBuilder<T> {
    fn default() -> Self {
        GlobMapBuilder {
            keys: vec![],
            values: vec![],
        }
    }
}

impl<T> GlobMapBuilder<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add<S: AsRef<str>>(&mut self, globstr: S, value: T) {
        self.keys.push(globstr.as_ref().to_string());
        self.values.push(value);
    }

    pub fn build(self) -> Result<GlobMap<T>, globset::Error> {
        let mut builder = GlobSetBuilder::new();
        for globstr in &self.keys {
            builder.add(GlobBuilder::new(globstr).literal_separator(true).build()?);
        }
        Ok(GlobMap {
            keys: self.keys,
            values: self.values,
            globset: builder.build()?,
        })
    }
}

#[derive(Debug)]
pub enum GlobMapResult<'a, T> {
    /// A unique matching pattern
    Found(String, &'a T),
    /// No matching pattern
    NotFound,
    /// Multiple matching patterns
    Ambiguous(Vec<(String, &'a T)>),
}

#[derive(Debug)]
pub enum GlobMapResultMut<'a, T> {
    /// A unique matching pattern
    Found(String, &'a mut T),
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
            GlobMapResult::Found(key.to_owned(), &self.values[outcome[0]])
        } else {
            // globset returns matched indices in ascending order, so walk one
            // iterator forward, advancing by the gap since the previous index
            // (`nth` consumes) rather than indexing the values repeatedly
            // (which `lookup_mut` cannot do: the vec is borrowed mutably).
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
            GlobMapResultMut::Found(key.to_owned(), &mut self.values[outcome[0]])
        } else {
            // globset returns matched indices in ascending order, so walk one
            // iterator forward, advancing by the gap since the previous index
            // (`nth` consumes) rather than indexing the values repeatedly
            // (which `lookup_mut` cannot do: the vec is borrowed mutably).
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
            GlobMapResult::Found(_, restricted) => {
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
                if !first_response.status().is_client_error() {
                    Ok(first_response)
                } else {
                    for (_, other_restricted) in items {
                        let other_response = other_restricted
                            .with_authentication(&client, renew_request)
                            .await?;
                        if !other_response.status().is_client_error() {
                            return Ok(other_response);
                        }
                    }
                    Ok(first_response)
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum StandardInnerAuthentication {
    HTTPBasicAuth(ForceHTTPBasicAuth),
    BearerAuth {
        auth: ForceBearerAuth,
        /// The `SYSAND_CRED_<LABEL>` stem this bearer came from, when it
        /// was built from environment variables. Display-only: publish
        /// auth failures name the variable to fix
        /// (design/credential-storage.md section 7).
        env_label: Option<Box<str>>,
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
            StandardInnerAuthentication::HTTPBasicAuth(inner) => {
                inner
                    .request_with_authentication(request, renew_request)
                    .await
            }
            StandardInnerAuthentication::BearerAuth { auth, .. } => {
                auth.request_with_authentication(request, renew_request)
                    .await
            }
        }
    }
}

/// Standard HTTP authentication policy where a restricted set of domains/paths have
/// BasicAuth username/password pairs specified, but they are sent only in response to a
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
/// upload auth failure can name the variable to fix
/// (design/credential-storage.md section 7).
#[derive(Clone)]
pub struct EnvBearerAuth {
    pub auth: ForceBearerAuth,
    /// The `SYSAND_CRED_<LABEL>` stem, when known.
    pub label: Option<String>,
}

impl std::fmt::Debug for EnvBearerAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is already redacted by `ForceBearerAuth`'s `Debug`;
        // hand-written only to show just the non-secret label.
        f.debug_struct("EnvBearerAuth")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl StandardHTTPAuthentication {
    /// Extracts the bearer tokens from the configured credential set into a URL-glob map
    /// suitable for driving publish-time credential selection. Basic-auth entries are
    /// dropped, since publish only supports bearer authentication.
    pub fn publish_bearer_auth_map(&self) -> Result<GlobMap<EnvBearerAuth>, globset::Error> {
        let mut partial = GlobMapBuilder::new();

        // `GlobMap` stores keys and values in parallel vectors. This clones the bearer
        // tokens into the publish-only map; an earlier version consumed `self` to move
        // them without cloning, but the lazy credential auth layer
        // (`CredentialStoreAuthentication`) holding this policy is not `Clone`, so
        // extraction must work by reference (see design/credential-storage.md,
        // section 9, which accepts the secret clones as the cost of that layer).
        for (key, sequence_auth) in self.restricted.keys.iter().zip(&self.restricted.values) {
            if let StandardInnerAuthentication::BearerAuth { auth, env_label } =
                &sequence_auth.lower
            {
                partial.add(
                    key,
                    EnvBearerAuth {
                        auth: auth.clone(),
                        label: env_label.as_deref().map(str::to_string),
                    },
                );
            }
        }

        partial.build()
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
    ) {
        self.partial.add(
            globstr,
            SequenceAuthentication {
                higher: Unauthenticated {},
                lower: StandardInnerAuthentication::HTTPBasicAuth(ForceHTTPBasicAuth {
                    username: username.as_ref().into(),
                    password: password.as_ref().into(),
                }),
            },
        );
    }

    pub fn add_bearer_auth<S: AsRef<str>, T: AsRef<str>>(&mut self, globstr: S, token: T) {
        self.push_bearer_auth(globstr, token, None);
    }

    /// Like [`Self::add_bearer_auth`], additionally recording the
    /// `SYSAND_CRED_<LABEL>` stem the credential came from, so publish
    /// auth failures can name the variable to fix.
    pub fn add_bearer_auth_labeled<S: AsRef<str>, T: AsRef<str>, L: AsRef<str>>(
        &mut self,
        globstr: S,
        token: T,
        env_label: L,
    ) {
        self.push_bearer_auth(globstr, token, Some(env_label.as_ref().into()));
    }

    fn push_bearer_auth<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        globstr: S,
        token: T,
        env_label: Option<Box<str>>,
    ) {
        self.partial.add(
            globstr,
            SequenceAuthentication {
                higher: Unauthenticated {},
                lower: StandardInnerAuthentication::BearerAuth {
                    auth: ForceBearerAuth::new(token),
                    env_label,
                },
            },
        );
    }
}

/// One stored credential as loaded into the lazy credential map: the bearer
/// token plus the non-secret record fields runtime messages need
/// (design/credential-storage.md sections 7 and 9): the index key names
/// the login in hints, and `expires_at` drives the reactive expiry hint
/// and publish's fail-fast check.
#[derive(Clone)]
pub struct StoredBearerAuth {
    auth: ForceBearerAuth,
    key: String,
    expires_at: Option<DateTime<Utc>>,
    /// Whether the reactive expiry hint has already been emitted for this
    /// record. `Arc`d so the flag is shared across the record's globs and
    /// across map clones: together with the once-per-process map cache
    /// this gives at most one warning per record per process.
    expiry_warned: Arc<AtomicBool>,
}

impl std::fmt::Debug for StoredBearerAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is already redacted by `ForceBearerAuth`'s `Debug`;
        // hand-written only to show the non-secret record fields and drop
        // the `expiry_warned` `Arc` noise.
        f.debug_struct("StoredBearerAuth")
            .field("key", &self.key)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl StoredBearerAuth {
    pub(crate) fn new(
        auth: ForceBearerAuth,
        key: String,
        expires_at: Option<DateTime<Utc>>,
    ) -> Self {
        StoredBearerAuth {
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
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }

    /// The reactive expiry hint (design/credential-storage.md section 9),
    /// returned at most once per record: `Some` exactly when the record
    /// carries a past `expires_at` and no hint was emitted before.
    fn take_expiry_warning(&self, now: DateTime<Utc>) -> Option<String> {
        let expires_at = self.expires_at?;
        if expires_at >= now || self.expiry_warned.swap(true, Ordering::SeqCst) {
            return None;
        }
        Some(format!(
            "credential for `{key}` may be expired or revoked; \
             re-authenticate to store a fresh credential",
            key = self.key
        ))
    }

    /// Emit the reactive expiry hint after a forced retry that still
    /// ended in a 4xx. Any 4xx counts, not just 401: GitLab-style hosts
    /// answer 404 on bad auth (design/credential-storage.md section 9).
    fn warn_if_expired(&self) {
        if let Some(message) = self.take_expiry_warning(Utc::now()) {
            log::warn!("{message}");
        }
    }

    #[cfg(test)]
    fn expiry_warning_emitted(&self) -> bool {
        self.expiry_warned.load(Ordering::SeqCst)
    }
}

/// Authentication layer that consults a persistent [`CredentialStore`] on
/// demand (design/credential-storage.md, section 9).
///
/// `inner` (the eager `SYSAND_CRED_*` env policy) runs first. Only when it
/// ends in a 4xx is the credential store read, at most once per process
/// (cached), and then:
///
/// - with no stored record matching the request URL, the inner response is
///   returned untouched, so a routine 404 on the resolve path costs no
///   extra request. This is why this is not a stock
///   [`SequenceAuthentication`]: its lower arm cannot see the higher arm's
///   response and would re-issue an identical request on every ordinary
///   404, permanently doubling round-trips for logged-in users;
/// - with a matching record, a *forced* authenticated retry is sent.
///   v1 stores bearer credentials only ([`CredentialScheme`]), so the
///   retry is always [`ForceBearerAuth`].
///
/// Escalation triggers on *any* 4xx (not just 401/403) because some hosts
/// (GitLab) answer 404 on missing or under-scoped auth. Requests that
/// succeed unauthenticated, and non-4xx failures, never touch the store.
///
/// The store lookup uses the initial request URL. As with
/// [`RestrictAuthentication`], a same-host redirect target is not
/// re-checked against the stored globs (reqwest forwards the header there)
/// and a cross-host redirect strips it.
pub struct CredentialStoreAuthentication<Inner, S> {
    inner: Inner,
    /// `None` when no store is available (for example, opening the default
    /// OS keyring store failed); behaves as an always-empty store.
    store: Option<Arc<S>>,
    /// Stored bearer credentials, read lazily. A `tokio` `OnceCell` (not a
    /// sync `OnceLock`) so concurrent first requests (the resolve path
    /// fans out with `join_all`) share a single read: at most one keychain
    /// touch per process.
    cache: tokio::sync::OnceCell<GlobMap<StoredBearerAuth>>,
}

/// Standard composed policy: eager `SYSAND_CRED_*` credentials first, then
/// lazily read stored credentials from `S`.
pub type StandardLazyHTTPAuthentication<S> =
    CredentialStoreAuthentication<StandardHTTPAuthentication, S>;

impl<Inner, S> std::fmt::Debug for CredentialStoreAuthentication<Inner, S>
where
    Inner: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hand-written to show `has_store`/`cache_read` rather than the
        // cache contents; the stored tokens it holds are already redacted
        // by `ForceBearerAuth`'s `Debug`.
        f.debug_struct("CredentialStoreAuthentication")
            .field("inner", &self.inner)
            .field("has_store", &self.store.is_some())
            .field("cache_read", &self.cache.get().is_some())
            .finish()
    }
}

impl<Inner, S> CredentialStoreAuthentication<Inner, S>
where
    S: CredentialStore + Send + Sync + 'static,
{
    pub fn new(inner: Inner, store: S) -> Self {
        CredentialStoreAuthentication {
            inner,
            store: Some(Arc::new(store)),
            cache: tokio::sync::OnceCell::new(),
        }
    }

    /// A policy with no credential store: only `inner` ever answers.
    /// For hosts where opening the store failed.
    pub fn without_store(inner: Inner) -> Self {
        CredentialStoreAuthentication {
            inner,
            store: None,
            cache: tokio::sync::OnceCell::new(),
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
                // The keyring crate is synchronous: a locked Linux Secret
                // Service can block on an unlock prompt and the store's
                // cross-process lock has a bounded (seconds-long) wait, so
                // the read runs on the blocking pool instead of stalling
                // sibling futures on the async worker. The blob is tiny
                // and this runs at most once per process.
                match tokio::task::spawn_blocking(move || read_stored_bearer_map(&*store)).await {
                    Ok(map) => map,
                    Err(err) => {
                        log::warn!(
                            "credential store read failed: {err}; \
                             continuing without stored credentials"
                        );
                        GlobMap::default()
                    }
                }
            })
            .await
    }

    /// Synchronous variant of [`Self::stored_bearer_map`] for callers
    /// outside the async runtime (publish's credential selection). Shares
    /// the same once-per-process cache; returns an owned map (cheap: the
    /// records hold `Arc`s and small strings, and publish calls this once).
    pub fn stored_bearer_map_blocking(&self) -> GlobMap<StoredBearerAuth> {
        if let Some(map) = self.cache.get() {
            return map.clone();
        }
        let map = match &self.store {
            None => GlobMap::default(),
            Some(store) => read_stored_bearer_map(store.as_ref()),
        };
        // On the current flow this never races the async accessor: publish
        // resolves its bearer only after discovery's `block_on` has
        // returned, and the runtime is current-thread, so `set` seeds the
        // cache for later callers. Should an async `get_or_init` ever be
        // in flight concurrently (e.g. on a multi-thread runtime), `set`
        // fails and the locally read map is returned as-is: the cost is
        // one extra store read, never a panic.
        let _ = self.cache.set(map.clone());
        map
    }
}

/// Read all stored records and build the URL-glob to bearer map.
///
/// Store errors degrade to "no stored credentials" for this process (the
/// result is cached, so at most one keychain touch and one warning): an
/// absent backend is the normal no-keyring case and only logged at debug
/// level, while a locked/denied backend is warned about with the unlock
/// and `SYSAND_CRED_*` remediations. Degrading (rather than aborting the
/// whole operation) matches design/credential-storage.md section 9: the
/// request itself may still succeed via env credentials or anonymously,
/// and hard failure is reserved for the `auth` commands.
fn read_stored_bearer_map<S: CredentialStore + ?Sized>(store: &S) -> GlobMap<StoredBearerAuth> {
    let records = match store.list() {
        Ok(records) => records,
        Err(CredentialStoreError::BackendAbsent { source }) => {
            log::debug!(
                "no OS keyring backend ({source}); \
                 using `SYSAND_CRED_*` credentials only"
            );
            return GlobMap::default();
        }
        Err(err) => {
            log::warn!(
                "could not read stored credentials: {err}; \
                 unlock your OS keyring, or provide credentials via \
                 `SYSAND_CRED_*` environment variables; \
                 continuing without stored credentials"
            );
            return GlobMap::default();
        }
    };

    let mut builder = GlobMapBuilder::new();
    for record in records {
        // Exhaustive on purpose: adding a scheme must revisit this map.
        match record.scheme {
            CredentialScheme::Bearer => {}
        }
        // One `StoredBearerAuth` per record, cloned per glob: the clones
        // share the record's expiry-warned flag.
        let auth = StoredBearerAuth::new(
            ForceBearerAuth::new(&record.secret),
            record.key.clone(),
            record.expires_at,
        );
        for glob in &record.globs {
            // Validate each glob individually so one invalid pattern skips
            // only itself, not every stored credential.
            if let Err(err) = GlobBuilder::new(glob).literal_separator(true).build() {
                log::warn!(
                    "ignoring invalid stored credential URL pattern `{glob}` for `{}`: {err}",
                    record.key
                );
                continue;
            }
            builder.add(glob, auth.clone());
        }
    }
    match builder.build() {
        Ok(map) => map,
        Err(err) => {
            log::warn!(
                "could not build stored credential URL patterns: {err}; \
                 continuing without stored credentials"
            );
            GlobMap::default()
        }
    }
}

impl<Inner, S> HTTPAuthentication for CredentialStoreAuthentication<Inner, S>
where
    Inner: HTTPAuthentication,
    S: CredentialStore + Send + Sync + 'static,
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

        if !initial_response.status().is_client_error() {
            return Ok(initial_response);
        }

        let stored = self.stored_bearer_map().await;
        let candidates = match stored.lookup(url.as_str()) {
            GlobMapResult::NotFound => return Ok(initial_response),
            GlobMapResult::Found(_, auth) => vec![auth],
            GlobMapResult::Ambiguous(items) => items.into_iter().map(|(_, auth)| auth).collect(),
        };

        // One stored credential covers several (normally non-overlapping) URL
        // patterns with the same token, so several patterns matching is a
        // real ambiguity only between *distinct* tokens. Records with
        // identical tokens collapse to the first record, so a hint after
        // a failed retry names that record's key.
        let mut deduped: Vec<&StoredBearerAuth> = Vec::new();
        for bearer in candidates {
            if !deduped.iter().any(|seen| seen.auth.0 == bearer.auth.0) {
                deduped.push(bearer);
            }
        }

        let mut deduped = deduped.into_iter();
        let first_bearer = deduped.next().expect("lookup produced no candidates");
        // A single distinct token retries once; genuinely ambiguous matches
        // try each in order until one yields a non-4xx response
        // (design/credential-storage.md, section 8), mirroring
        // `RestrictAuthentication`. If every candidate fails, the first
        // retry response is returned. The single-match case is this same
        // path with an empty remainder loop.
        if deduped.len() == 0 {
            log::debug!("stored credential matches `{url}`; retrying with forced bearer auth");
        } else {
            log::warn!("URL {url} matches multiple stored credentials; trying each in order");
        }
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

#[cfg(test)]
#[path = "./auth_tests.rs"]
mod tests;
