// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::auth::{
    CredentialStoreAuthentication, GlobMapBuilder, GlobMapResultMut, HTTPAuthentication,
    StandardHTTPAuthentication, StandardHTTPAuthenticationBuilder,
};
use crate::credential_store::keyring_store::{BlobBackend, LockedBlobStore};
use crate::credential_store::test_support::{InMemoryBlobBackend, unique_lock_path};
use crate::credential_store::{
    CredentialBlob, CredentialRecord, CredentialScheme, CredentialStoreError, serialize_blob,
};
use crate::resolve::net_utils::create_reqwest_client;

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

#[test]
fn globmap_matches_template_expanded_urls() -> Result<(), Box<dyn std::error::Error>> {
    // The credential glob a user configures for a templated index
    // (`SYSAND_CRED_X=https://gitlab.com/api/v4/projects/123/**`) must
    // match the expanded request URLs, where the file path sits mid-URL
    // percent-encoded and a query string follows.
    // This glob only fits the *expanded* request URL: it matches on the
    // percent-encoded file path (`admin%2F...`), which does not exist in
    // the raw template (where the placeholder still reads `{path}`).
    let mut builder = GlobMapBuilder::new();
    builder.add(
        "https://gitlab.com/api/v4/projects/123/repository/files/admin%2F*/raw?ref=main",
        1,
    );
    let mut globmap = builder.build()?;

    let template = crate::index_location::IndexLocation::parse(
        "https://gitlab.com/api/v4/projects/123/repository/files/{path}/raw?ref=main",
    )?;
    let expanded = template.resolve("admin/proj0/versions.json".split('/'));
    assert_eq!(
        expanded.as_str(),
        "https://gitlab.com/api/v4/projects/123/repository/files/\
         admin%2Fproj0%2Fversions.json/raw?ref=main"
    );

    // The raw template does not match the glob...
    assert!(matches!(
        globmap.lookup_mut(&template.to_string()),
        GlobMapResultMut::NotFound
    ));
    // ...but the expanded request URL does.
    if let GlobMapResultMut::Found(_, val) = globmap.lookup_mut(expanded.as_str()) {
        assert_eq!(*val, 1);
    } else {
        panic!("expected credential glob to match expanded template URL");
    }

    Ok(())
}

// select_bearer: the selection semantics shared by the runtime read
// retry, whoami, and publish. The end-to-end behavior of each consumer is
// pinned by its own tests; these cover the helper's collapse rules
// directly, in particular the mixed identical-plus-distinct case only
// the runtime's try-all path can observe.
mod select_bearer {
    use crate::auth::{BearerSelection, GlobMap, GlobMapBuilder, select_bearer};

    const URL: &str = "https://example.com/api/v1/upload";

    fn map(entries: &[(&str, &str)]) -> GlobMap<String> {
        let mut builder = GlobMapBuilder::new();
        for (pattern, token) in entries {
            builder.add(*pattern, (*token).to_string());
        }
        builder.build().unwrap()
    }

    fn select(map: &GlobMap<String>) -> BearerSelection<'_, String> {
        select_bearer(map, URL, String::as_str)
    }

    #[test]
    fn no_matching_pattern_is_none() {
        let map = map(&[("https://other.example/**", "tok")]);
        assert!(matches!(select(&map), BearerSelection::None));
    }

    #[test]
    fn a_unique_match_is_unique() {
        let map = map(&[("https://example.com/**", "tok")]);
        assert!(matches!(select(&map), BearerSelection::Unique(tok) if tok == "tok"));
    }

    #[test]
    fn identical_token_candidates_collapse_to_the_first() {
        let map = map(&[
            ("https://example.com/**", "same"),
            ("https://example.com/api/**", "same"),
        ]);
        assert!(matches!(select(&map), BearerSelection::Unique(tok) if tok == "same"));
    }

    #[test]
    fn mixed_candidates_dedupe_in_map_order_and_count_all_matches() {
        // Two patterns of one credential plus a distinct one: the error
        // count reports all three matches, while try-all consumers walk
        // the two distinct tokens in map order.
        let map = map(&[
            ("https://example.com/**", "tok-a"),
            ("https://example.com/api/**", "tok-a"),
            ("https://example.com/api/v1/**", "tok-b"),
        ]);
        match select(&map) {
            BearerSelection::Ambiguous {
                candidates: 3,
                deduped,
            } => {
                let tokens: Vec<&str> = deduped.iter().map(|token| token.as_str()).collect();
                assert_eq!(tokens, ["tok-a", "tok-b"]);
            }
            other => panic!("expected a three-candidate ambiguity, got {other:?}"),
        }
    }
}

#[test]
fn publish_bearer_auth_map_keeps_bearer_drops_basic_and_carries_labels()
-> Result<(), Box<dyn std::error::Error>> {
    let mut builder = crate::auth::StandardHTTPAuthenticationBuilder::new();
    builder.add_basic_auth("https://basic.example.com/*", "user", "password");
    builder.add_bearer_auth("https://bearer.example.com/*", "tok");
    builder.add_bearer_auth_labeled("https://labeled.example.com/*", "tok2", "TEAMIDX");
    let policy = builder.build()?;

    // By-ref extraction: the policy stays usable afterwards.
    let bearer_map = policy.publish_bearer_auth_map()?;

    if let crate::auth::GlobMapResult::Found(_, entry) =
        bearer_map.lookup("https://bearer.example.com/upload")
    {
        assert_eq!(&*entry.auth.0, "tok");
        assert_eq!(entry.label, None);
    } else {
        panic!("expected bearer entry to be extracted");
    }

    if let crate::auth::GlobMapResult::Found(_, entry) =
        bearer_map.lookup("https://labeled.example.com/upload")
    {
        assert_eq!(&*entry.auth.0, "tok2");
        assert_eq!(entry.label.as_deref(), Some("TEAMIDX"));
    } else {
        panic!("expected labeled bearer entry to be extracted");
    }

    assert!(matches!(
        bearer_map.lookup("https://basic.example.com/upload"),
        crate::auth::GlobMapResult::NotFound
    ));
    assert!(matches!(
        bearer_map.lookup("https://other.example.com/upload"),
        crate::auth::GlobMapResult::NotFound
    ));

    Ok(())
}

// Tests for the lazy credential store layer
// (`CredentialStoreAuthentication`).

/// A backend that counts its reads, so tests can assert when (and how
/// often) the credential store is actually read (one backend read per
/// store `list`).
#[derive(Debug, Clone)]
struct CountingBackend {
    inner: InMemoryBlobBackend,
    reads: Arc<AtomicUsize>,
}

impl BlobBackend for CountingBackend {
    fn read(&self) -> Result<Option<String>, CredentialStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read()
    }

    fn write(&self, raw: &str) -> Result<(), CredentialStoreError> {
        self.inner.write(raw)
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        self.inner.delete()
    }
}

/// A store holding exactly `records`, counting its backend reads.
fn counting_store(
    records: Vec<CredentialRecord>,
) -> (LockedBlobStore<CountingBackend>, Arc<AtomicUsize>) {
    let raw = serialize_blob(&CredentialBlob::new(records));
    let reads = Arc::new(AtomicUsize::new(0));
    let backend = CountingBackend {
        inner: InMemoryBlobBackend::with_contents(&raw),
        reads: reads.clone(),
    };
    (LockedBlobStore::new(backend, unique_lock_path()), reads)
}

/// A backend whose reads always fail, for the degrade-to-no-credentials
/// paths. `absent` selects `BackendAbsent` over `BackendDenied` (both are
/// warned about); the request-level behavior must be identical.
#[derive(Debug)]
struct FailingBackend {
    absent: bool,
    reads: Arc<AtomicUsize>,
}

impl FailingBackend {
    fn error(&self) -> CredentialStoreError {
        if self.absent {
            CredentialStoreError::BackendAbsent {
                source: "no keyring backend".into(),
            }
        } else {
            CredentialStoreError::BackendDenied {
                source: "keyring is locked".into(),
            }
        }
    }
}

impl BlobBackend for FailingBackend {
    fn read(&self) -> Result<Option<String>, CredentialStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Err(self.error())
    }

    fn write(&self, _raw: &str) -> Result<(), CredentialStoreError> {
        Err(self.error())
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        Err(self.error())
    }
}

fn failing_store(absent: bool) -> (LockedBlobStore<FailingBackend>, Arc<AtomicUsize>) {
    let reads = Arc::new(AtomicUsize::new(0));
    let backend = FailingBackend {
        absent,
        reads: reads.clone(),
    };
    (LockedBlobStore::new(backend, unique_lock_path()), reads)
}

fn bearer_record(globs: &[String], secret: &str) -> CredentialRecord {
    CredentialRecord {
        key: "https://example.com/".to_string(),
        globs: globs.to_vec(),
        scheme: CredentialScheme::Bearer,
        secret: secret.to_string(),
        expires_at: None,
        subject: None,
        token_name: None,
        token_prefix: None,
        validated: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

fn empty_env_policy() -> StandardHTTPAuthentication {
    StandardHTTPAuthenticationBuilder::new().build().unwrap()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap()
}

/// Drive one GET for `url` through `policy`, like the resolve path does.
fn get<P: HTTPAuthentication>(
    runtime: &tokio::runtime::Runtime,
    policy: &P,
    url: &str,
) -> reqwest::Response {
    let client = create_reqwest_client().unwrap();
    let url = url.to_string();
    let renew = move |c: &reqwest_middleware::ClientWithMiddleware| c.get(&url);
    runtime
        .block_on(policy.with_authentication(&client, &renew))
        .unwrap()
}

/// One GET for `server`'s `/pkg/versions.json` through `policy`.
fn get_versions<P: HTTPAuthentication>(
    runtime: &tokio::runtime::Runtime,
    policy: &P,
    server: &mockito::Server,
) -> reqwest::Response {
    get(
        runtime,
        policy,
        &format!("{}/pkg/versions.json", server.url()),
    )
}

/// Mock `GET /pkg/versions.json`: with `bearer` the request must carry
/// exactly that token; without, it must carry no credential at all.
/// Answered with `status` for exactly `hits` requests.
fn versions_mock(
    server: &mut mockito::Server,
    bearer: Option<&str>,
    status: usize,
    hits: usize,
) -> mockito::Mock {
    let mock = server.mock("GET", "/pkg/versions.json");
    let mock = match bearer {
        None => mock.match_header("authorization", mockito::Matcher::Missing),
        Some(token) => mock.match_header("authorization", format!("Bearer {token}").as_str()),
    };
    mock.with_status(status).expect(hits).create()
}

#[test]
fn lazy_layer_never_reads_store_on_success_server_error_or_rate_limiting() {
    // 2xx needs no credential, 5xx is not an auth verdict, and 429 is
    // rate limiting, never a verdict: each response is returned as-is,
    // with no store read and no forced retry (expect(1) asserts that; a
    // retry against a host that just throttled us would spend more of the
    // rate budget).
    for status in [200, 500, 429] {
        let mut server = mockito::Server::new();
        let mock = versions_mock(&mut server, None, status, 1);

        let (store, lists) = counting_store(vec![bearer_record(
            &[format!("{}/**", server.url())],
            "stored-token",
        )]);
        let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
        let runtime = runtime();

        let response = get(
            &runtime,
            &policy,
            &format!("{}/pkg/versions.json", server.url()),
        );

        assert_eq!(response.status().as_u16(), status as u16);
        assert_eq!(lists.load(Ordering::SeqCst), 0, "status = {status}");
        mock.assert();
    }
}

#[test]
fn sequence_auth_does_not_try_lower_on_rate_limiting() {
    // The same 429 carve-out in `SequenceAuthentication`: a rate-limited
    // unauthenticated request must not be retried with the env credential.
    let mut server = mockito::Server::new();
    let mock = versions_mock(&mut server, None, 429, 1);
    let bearer_mock = versions_mock(&mut server, Some("env-token"), 200, 0);

    let mut env_builder = StandardHTTPAuthenticationBuilder::new();
    env_builder.add_bearer_auth(format!("{}/**", server.url()), "env-token");
    let policy = env_builder.build().unwrap();
    let runtime = runtime();

    let response = get_versions(&runtime, &policy, &server);

    assert_eq!(response.status().as_u16(), 429);
    mock.assert();
    bearer_mock.assert();
}

#[test]
fn lazy_layer_reads_store_once_and_returns_original_response_on_no_match() {
    let mut server = mockito::Server::new();
    // Routine 404s on the resolve path: the store is read exactly once
    // across requests, and with no matching record each original response
    // comes back untouched with no extra request (expect(2) with two
    // `get`s asserts no retries happen).
    let mock = versions_mock(&mut server, None, 404, 2);

    let (store, lists) = counting_store(vec![bearer_record(
        &["https://other.example.com/**".to_string()],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();
    let url = format!("{}/pkg/versions.json", server.url());

    let first = get(&runtime, &policy, &url);
    let second = get(&runtime, &policy, &url);

    assert_eq!(first.status().as_u16(), 404);
    assert_eq!(second.status().as_u16(), 404);
    assert_eq!(lists.load(Ordering::SeqCst), 1);
    mock.assert();
}

#[test]
fn lazy_layer_matching_record_forces_authenticated_retry() {
    let mut server = mockito::Server::new();
    let unauth_mock = versions_mock(&mut server, None, 401, 1);
    let bearer_mock = versions_mock(&mut server, Some("stored-token"), 200, 1);

    let (store, lists) = counting_store(vec![bearer_record(
        &[format!("{}/**", server.url())],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();

    let response = get_versions(&runtime, &policy, &server);

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(lists.load(Ordering::SeqCst), 1);
    unauth_mock.assert();
    bearer_mock.assert();
}

#[test]
fn lazy_layer_env_credential_wins_without_store_read() {
    let mut server = mockito::Server::new();
    let unauth_mock = versions_mock(&mut server, None, 401, 1);
    let env_mock = versions_mock(&mut server, Some("env-token"), 200, 1);

    let mut env_builder = StandardHTTPAuthenticationBuilder::new();
    env_builder.add_bearer_auth(format!("{}/**", server.url()), "env-token");
    let (store, lists) = counting_store(vec![bearer_record(
        &[format!("{}/**", server.url())],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(env_builder.build().unwrap(), store);
    let runtime = runtime();

    let response = get_versions(&runtime, &policy, &server);

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(lists.load(Ordering::SeqCst), 0);
    unauth_mock.assert();
    env_mock.assert();
}

#[test]
fn lazy_layer_failed_env_credential_escalates_to_store() {
    let mut server = mockito::Server::new();
    let unauth_mock = versions_mock(&mut server, None, 401, 1);
    let env_mock = versions_mock(&mut server, Some("stale-env-token"), 401, 1);
    let stored_mock = versions_mock(&mut server, Some("stored-token"), 200, 1);

    let mut env_builder = StandardHTTPAuthenticationBuilder::new();
    env_builder.add_bearer_auth(format!("{}/**", server.url()), "stale-env-token");
    let (store, lists) = counting_store(vec![bearer_record(
        &[format!("{}/**", server.url())],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(env_builder.build().unwrap(), store);
    let runtime = runtime();

    let response = get_versions(&runtime, &policy, &server);

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(lists.load(Ordering::SeqCst), 1);
    unauth_mock.assert();
    env_mock.assert();
    stored_mock.assert();
}

#[test]
fn lazy_layer_overlapping_globs_of_one_record_retry_once() {
    let mut server = mockito::Server::new();
    let unauth_mock = versions_mock(&mut server, None, 401, 1);
    // One login covering the URL with two patterns is not an ambiguity:
    // exactly one forced retry with its token.
    let bearer_mock = versions_mock(&mut server, Some("stored-token"), 200, 1);

    let (store, _lists) = counting_store(vec![bearer_record(
        &[
            format!("{}/**", server.url()),
            format!("{}/pkg/*", server.url()),
        ],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();

    let response = get_versions(&runtime, &policy, &server);

    assert_eq!(response.status().as_u16(), 200);
    unauth_mock.assert();
    bearer_mock.assert();
}

#[test]
fn lazy_layer_ambiguous_distinct_records_try_all_in_order() {
    let mut server = mockito::Server::new();
    let unauth_mock = versions_mock(&mut server, None, 401, 1);
    let rejected_mock = versions_mock(&mut server, Some("broad-token"), 401, 1);
    let accepted_mock = versions_mock(&mut server, Some("narrow-token"), 200, 1);

    let mut broad = bearer_record(&[format!("{}/**", server.url())], "broad-token");
    broad.key = "https://example.com/broad/".to_string();
    let mut narrow = bearer_record(&[format!("{}/pkg/*", server.url())], "narrow-token");
    narrow.key = "https://example.com/narrow/".to_string();
    let (store, _lists) = counting_store(vec![broad, narrow]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();

    let response = get_versions(&runtime, &policy, &server);

    assert_eq!(response.status().as_u16(), 200);
    unauth_mock.assert();
    rejected_mock.assert();
    accepted_mock.assert();
}

#[test]
fn lazy_layer_invalid_stored_glob_skips_only_that_pattern() {
    let mut server = mockito::Server::new();
    let unauth_mock = versions_mock(&mut server, None, 401, 1);
    let bearer_mock = versions_mock(&mut server, Some("stored-token"), 200, 1);

    let (store, _lists) = counting_store(vec![bearer_record(
        &["[invalid".to_string(), format!("{}/**", server.url())],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();

    let response = get_versions(&runtime, &policy, &server);

    assert_eq!(response.status().as_u16(), 200);
    unauth_mock.assert();
    bearer_mock.assert();
}

#[test]
fn lazy_layer_store_errors_degrade_to_no_credentials() {
    for absent in [true, false] {
        let mut server = mockito::Server::new();
        let mock = versions_mock(&mut server, None, 404, 2);

        let (store, lists) = failing_store(absent);
        let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
        let runtime = runtime();
        let url = format!("{}/pkg/versions.json", server.url());

        // Both error kinds behave as "no stored credentials" for the
        // request, and the failed read is cached like a successful one.
        let first = get(&runtime, &policy, &url);
        let second = get(&runtime, &policy, &url);

        assert_eq!(first.status().as_u16(), 404, "absent = {absent}");
        assert_eq!(second.status().as_u16(), 404, "absent = {absent}");
        assert_eq!(lists.load(Ordering::SeqCst), 1, "absent = {absent}");
        mock.assert();
    }
}

#[test]
fn lazy_layer_without_store_returns_original_response() {
    let mut server = mockito::Server::new();
    let mock = versions_mock(&mut server, None, 404, 1);

    let policy: CredentialStoreAuthentication<_, InMemoryBlobBackend> =
        CredentialStoreAuthentication::without_store(empty_env_policy());
    let runtime = runtime();

    let response = get_versions(&runtime, &policy, &server);

    assert_eq!(response.status().as_u16(), 404);
    mock.assert();
}

#[test]
fn direct_stored_bearer_read_does_exactly_one_store_read_per_call() {
    let (store, lists) = counting_store(vec![bearer_record(
        &["https://other.example.com/**".to_string()],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);

    // Publish-style direct read: one `list` per call, no cache involved.
    let map = policy.read_stored_bearer_map_direct();
    assert!(matches!(
        map.lookup("https://other.example.com/upload"),
        crate::auth::GlobMapResult::Found(_, _)
    ));
    assert_eq!(lists.load(Ordering::SeqCst), 1);
}

#[test]
fn direct_stored_bearer_read_degrades_store_errors_to_an_empty_map() {
    for absent in [true, false] {
        let (store, lists) = failing_store(absent);
        let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);

        let map = policy.read_stored_bearer_map_direct();
        assert!(
            matches!(
                map.lookup("https://example.com/upload"),
                crate::auth::GlobMapResult::NotFound
            ),
            "absent = {absent}"
        );
        assert_eq!(lists.load(Ordering::SeqCst), 1, "absent = {absent}");
    }
}

// Debug redaction: the hand-written `Debug` impls exist so a secret can
// never reach logs via an accidental `{:?}`; pin that here.

#[test]
fn debug_never_renders_secrets() {
    let bearer = crate::auth::ForceBearerAuth::new("bearer-secret");
    let rendered = format!("{bearer:?}");
    assert!(rendered.contains("<redacted>"), "rendered: {rendered}");
    assert!(!rendered.contains("bearer-secret"), "rendered: {rendered}");

    let basic = crate::auth::ForceHTTPBasicAuth {
        username: "alice".into(),
        password: "basic-secret".into(),
    };
    let rendered = format!("{basic:?}");
    assert!(rendered.contains("alice"), "rendered: {rendered}");
    assert!(rendered.contains("<redacted>"), "rendered: {rendered}");
    assert!(!rendered.contains("basic-secret"), "rendered: {rendered}");
    let env = crate::auth::EnvBearerAuth {
        auth: crate::auth::ForceBearerAuth::new("env-secret"),
        label: Some("TEAMIDX".to_string()),
    };
    let rendered = format!("{env:?}");
    assert!(rendered.contains("TEAMIDX"), "rendered: {rendered}");
    assert!(!rendered.contains("env-secret"), "rendered: {rendered}");

    let stored = crate::auth::StoredBearerAuth::new(
        crate::auth::ForceBearerAuth::new("stored-secret"),
        "https://example.com/".to_string(),
        None,
    );
    let rendered = format!("{stored:?}");
    assert!(
        rendered.contains("https://example.com/"),
        "rendered: {rendered}"
    );
    assert!(!rendered.contains("stored-secret"), "rendered: {rendered}");

    // The composed policy: env credentials in `inner`, stored credentials
    // in the populated cache. Neither secret may surface.
    let mut env_builder = StandardHTTPAuthenticationBuilder::new();
    env_builder.add_bearer_auth("https://bearer.example.com/**", "env-secret");
    let (store, _lists) = counting_store(vec![bearer_record(
        &["https://other.example.com/**".to_string()],
        "stored-secret",
    )]);
    let policy = CredentialStoreAuthentication::new(env_builder.build().unwrap(), store);
    runtime().block_on(policy.stored_bearer_map());
    let rendered = format!("{policy:?}");
    assert!(rendered.contains("<redacted>"), "rendered: {rendered}");
    assert!(!rendered.contains("env-secret"), "rendered: {rendered}");
    assert!(!rendered.contains("stored-secret"), "rendered: {rendered}");
}

// The reactive expiry hint.

use chrono::{Duration, Utc};

use crate::auth::{ForceBearerAuth, StoredBearerAuth};

#[test]
fn stored_bearer_expiry_warning_fires_at_most_once() {
    let now = Utc::now();
    let bearer = StoredBearerAuth::new(
        ForceBearerAuth::new("tok"),
        "https://example.com/".to_string(),
        Some(now - Duration::hours(1)),
    );

    let message = bearer
        .take_expiry_warning(now)
        .expect("an expired record must produce the hint once");
    assert!(
        message.contains("credential for `https://example.com/` may be expired or revoked"),
        "message: {message}"
    );
    // The core warning names no CLI command; it says to re-authenticate.
    assert!(
        message.contains("re-authenticate to store a fresh credential"),
        "message: {message}"
    );
    assert!(!message.contains("sysand auth"), "message: {message}");
    // Second failure on the same record: no repeat hint.
    assert_eq!(bearer.take_expiry_warning(now), None);
}

#[test]
fn stored_bearer_expiry_warning_skips_unexpired_and_unknown_expiry() {
    let now = Utc::now();
    let unexpired = StoredBearerAuth::new(
        ForceBearerAuth::new("tok"),
        "https://example.com/".to_string(),
        Some(now + Duration::hours(1)),
    );
    assert_eq!(unexpired.take_expiry_warning(now), None);
    assert!(!unexpired.expiry_warning_emitted());

    let unknown = StoredBearerAuth::new(
        ForceBearerAuth::new("tok"),
        "https://example.com/".to_string(),
        None,
    );
    assert_eq!(unknown.take_expiry_warning(now), None);
    assert!(!unknown.expiry_warning_emitted());
}

/// Look the stored bearer for `url` up in the policy's cached map, to
/// observe the per-record expiry-warned flag after driving requests. Must
/// go through the request path's cache (not a direct store read, which
/// would build fresh records with fresh flags); the clone shares the
/// record's `expiry_warned` flag (it is an `Arc`).
fn stored_bearer_for<Inner, B>(
    runtime: &tokio::runtime::Runtime,
    policy: &CredentialStoreAuthentication<Inner, B>,
    url: &str,
) -> StoredBearerAuth
where
    B: BlobBackend + Send + Sync + 'static,
{
    match runtime.block_on(policy.stored_bearer_map()).lookup(url) {
        crate::auth::GlobMapResult::Found(_, bearer) => bearer.clone(),
        other => panic!("expected a stored bearer for `{url}`, got {other:?}"),
    }
}

#[test]
fn lazy_layer_warns_once_for_an_expired_record_that_keeps_failing() {
    let mut server = mockito::Server::new();
    // GitLab-style host: bad auth answers 404, not 401 (the hint must
    // fire on any 4xx). Two full escalations; the flag must latch after
    // the first, so the hint is emitted at most once per process.
    let unauth_mock = versions_mock(&mut server, None, 404, 2);
    let forced_mock = versions_mock(&mut server, Some("stored-token"), 404, 2);

    let mut record = bearer_record(&[format!("{}/**", server.url())], "stored-token");
    record.expires_at = Some(Utc::now() - Duration::days(1));
    let (store, _lists) = counting_store(vec![record]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();
    let url = format!("{}/pkg/versions.json", server.url());

    let first = get(&runtime, &policy, &url);
    // The hint was emitted (and latched) during the first escalation, so
    // the second identical failure cannot repeat it: `take_expiry_warning`
    // returns `None` once the flag is set (covered by the unit test above).
    assert!(stored_bearer_for(&runtime, &policy, &url).expiry_warning_emitted());
    let second = get(&runtime, &policy, &url);

    assert_eq!(first.status().as_u16(), 404);
    assert_eq!(second.status().as_u16(), 404);
    assert!(stored_bearer_for(&runtime, &policy, &url).expiry_warning_emitted());
    unauth_mock.assert();
    forced_mock.assert();
}

#[test]
fn lazy_layer_does_not_warn_for_an_unexpired_record_or_a_successful_retry() {
    // Case 1: unexpired record, failing retry: no hint.
    let mut server = mockito::Server::new();
    let _unauth = versions_mock(&mut server, None, 401, 1);
    let _forced = versions_mock(&mut server, Some("stored-token"), 404, 1);
    let mut record = bearer_record(&[format!("{}/**", server.url())], "stored-token");
    record.expires_at = Some(Utc::now() + Duration::days(1));
    let (store, _lists) = counting_store(vec![record]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();
    let url = format!("{}/pkg/versions.json", server.url());
    get(&runtime, &policy, &url);
    assert!(!stored_bearer_for(&runtime, &policy, &url).expiry_warning_emitted());

    // Case 2: expired record, but the forced retry succeeds: no hint
    // (the credential demonstrably still works).
    let mut server = mockito::Server::new();
    let _unauth = versions_mock(&mut server, None, 401, 1);
    let _forced = versions_mock(&mut server, Some("stored-token"), 200, 1);
    let mut record = bearer_record(&[format!("{}/**", server.url())], "stored-token");
    record.expires_at = Some(Utc::now() - Duration::days(1));
    let (store, _lists) = counting_store(vec![record]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let url = format!("{}/pkg/versions.json", server.url());
    let response = get(&runtime, &policy, &url);
    assert_eq!(response.status().as_u16(), 200);
    assert!(!stored_bearer_for(&runtime, &policy, &url).expiry_warning_emitted());
}
