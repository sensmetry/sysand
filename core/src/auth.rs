// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! This module includes utilities for creating and using authentication policies for requests.

use globset::{GlobBuilder, GlobSetBuilder};
use reqwest::{Response, header};
use reqwest_middleware::{ClientWithMiddleware, RequestBuilder};

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
#[derive(Debug, Clone)]
pub struct ForceHTTPBasicAuth {
    pub username: Box<str>,
    pub password: Box<str>,
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
#[derive(Debug, Clone)]
pub struct ForceBearerAuth(Box<str>);

impl ForceBearerAuth {
    pub fn new<S: AsRef<str>>(token: S) -> ForceBearerAuth {
        Self(token.as_ref().into())
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
            // Need to do some magic to keep multiple (disjoint) references into a mutable array
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
            // Need to do some magic to keep multiple (disjoint) references into a mutable array
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
    BearerAuth(ForceBearerAuth),
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
            StandardInnerAuthentication::BearerAuth(inner) => {
                inner
                    .request_with_authentication(request, renew_request)
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
        self.partial.add(
            globstr,
            SequenceAuthentication {
                higher: Unauthenticated {},
                lower: StandardInnerAuthentication::BearerAuth(ForceBearerAuth::new(token)),
            },
        );
    }

    // TODO: For other authentication schemes
    // pub fn add_..._auth<S: AsRef<str>, ...>(&self, globstr: S, ...)
}

// pub struct GlobsetAuth
#[cfg(test)]
mod tests {
    use crate::auth::{GlobMapBuilder, GlobMapResultMut};

    #[test]
    fn basic_globmap_lookup() -> Result<(), Box<dyn std::error::Error>> {
        let mut builder = GlobMapBuilder::new();
        builder.add("a*.com/*", 1);
        builder.add("a*.com/**", 2);
        builder.add("b.com/*", 3);
        builder.add("a*.com/*/*", 4);
        let mut globmap = builder.build()?;

        if let GlobMapResultMut::Ambiguous(vals) = globmap.lookup_mut("axx.com/xxx") {
            let vals: Vec<i32> = vals.into_iter().map(|(_, i)| *i).collect();
            assert_eq!(vals, vec![1, 2]);
        } else {
            panic!("Expected ambiguous result.");
        }

        if let GlobMapResultMut::Ambiguous(vals) = globmap.lookup_mut("axx.com/xxx/xxx") {
            let vals: Vec<i32> = vals.into_iter().map(|(_, i)| *i).collect();
            assert_eq!(vals, vec![2, 4]);
        } else {
            panic!("Expected ambiguous result.");
        }

        let key = "axx.com/xxx/xxx/xxx";
        if let GlobMapResultMut::Found(k, val) = globmap.lookup_mut(key) {
            assert_eq!(k, key);
            assert_eq!(*val, 2);
        } else {
            panic!("Expected unambiguous result.");
        }

        let key = "b.com/xxx";
        if let GlobMapResultMut::Found(k, val) = globmap.lookup_mut(key) {
            assert_eq!(k, key);
            assert_eq!(*val, 3);
        } else {
            panic!("Expected unambiguous result.");
        }

        if let GlobMapResultMut::NotFound = globmap.lookup_mut("axx.com") {
        } else {
            panic!("Expected no result.");
        }

        if let GlobMapResultMut::NotFound = globmap.lookup_mut("bxx.com/xxx") {
        } else {
            panic!("Expected no result.");
        }

        if let GlobMapResultMut::NotFound = globmap.lookup_mut("cxx.com/xxx") {
        } else {
            panic!("Expected no result.");
        }

        Ok(())
    }
}
