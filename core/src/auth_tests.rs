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
use crate::credential_store::{
    CredentialRecord, CredentialScheme, CredentialStore, CredentialStoreError,
    InMemoryCredentialStore,
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

#[test]
fn publish_bearer_auth_map_keeps_bearer_drops_basic() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = crate::auth::StandardHTTPAuthenticationBuilder::new();
    builder.add_basic_auth("https://basic.example.com/*", "user", "password");
    builder.add_bearer_auth("https://bearer.example.com/*", "tok");
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

#[test]
fn publish_bearer_auth_map_carries_the_env_label() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = crate::auth::StandardHTTPAuthenticationBuilder::new();
    builder.add_bearer_auth_labeled("https://bearer.example.com/*", "tok", "TEAMIDX");
    let policy = builder.build()?;

    let bearer_map = policy.publish_bearer_auth_map()?;

    if let crate::auth::GlobMapResult::Found(_, entry) =
        bearer_map.lookup("https://bearer.example.com/upload")
    {
        assert_eq!(&*entry.auth.0, "tok");
        assert_eq!(entry.label.as_deref(), Some("TEAMIDX"));
    } else {
        panic!("expected labeled bearer entry to be extracted");
    }

    Ok(())
}

// Tests for the lazy credential store layer
// (`CredentialStoreAuthentication`).

/// Counts `list` calls so tests can assert when (and how often) the
/// credential store is actually read.
#[derive(Debug)]
struct CountingStore {
    inner: InMemoryCredentialStore,
    lists: Arc<AtomicUsize>,
}

impl CountingStore {
    fn with_records(records: Vec<CredentialRecord>) -> (Self, Arc<AtomicUsize>) {
        let mut inner = InMemoryCredentialStore::new();
        for record in records {
            inner.upsert(record).unwrap();
        }
        let lists = Arc::new(AtomicUsize::new(0));
        (
            CountingStore {
                inner,
                lists: lists.clone(),
            },
            lists,
        )
    }
}

impl CredentialStore for CountingStore {
    fn list(&self) -> Result<Vec<CredentialRecord>, CredentialStoreError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        self.inner.list()
    }

    fn upsert(&mut self, record: CredentialRecord) -> Result<(), CredentialStoreError> {
        self.inner.upsert(record)
    }

    fn remove(&mut self, key: &str) -> Result<bool, CredentialStoreError> {
        self.inner.remove(key)
    }
}

/// A store whose reads always fail, for the degrade-to-no-credentials
/// paths. `absent` selects `BackendAbsent` (quiet) over `BackendDenied`
/// (warned about); the request-level behavior must be identical.
#[derive(Debug)]
struct FailingStore {
    absent: bool,
    lists: Arc<AtomicUsize>,
}

impl FailingStore {
    fn new(absent: bool) -> (Self, Arc<AtomicUsize>) {
        let lists = Arc::new(AtomicUsize::new(0));
        (
            FailingStore {
                absent,
                lists: lists.clone(),
            },
            lists,
        )
    }

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

impl CredentialStore for FailingStore {
    fn list(&self) -> Result<Vec<CredentialRecord>, CredentialStoreError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        Err(self.error())
    }

    fn upsert(&mut self, _record: CredentialRecord) -> Result<(), CredentialStoreError> {
        Err(self.error())
    }

    fn remove(&mut self, _key: &str) -> Result<bool, CredentialStoreError> {
        Err(self.error())
    }
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

#[test]
fn lazy_layer_never_reads_store_on_unauthenticated_success() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/pkg/versions.json")
        .with_status(200)
        .expect(1)
        .create();

    let (store, lists) = CountingStore::with_records(vec![bearer_record(
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

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(lists.load(Ordering::SeqCst), 0);
    mock.assert();
}

#[test]
fn lazy_layer_never_reads_store_on_server_error() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/pkg/versions.json")
        .with_status(500)
        .expect(1)
        .create();

    let (store, lists) = CountingStore::with_records(vec![bearer_record(
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

    assert_eq!(response.status().as_u16(), 500);
    assert_eq!(lists.load(Ordering::SeqCst), 0);
    mock.assert();
}

#[test]
fn lazy_layer_reads_store_once_and_returns_original_response_on_no_match() {
    let mut server = mockito::Server::new();
    // Routine 404s on the resolve path: the store is read exactly once
    // across requests, and with no matching record each original response
    // comes back untouched with no extra request (expect(2) with two
    // `get`s asserts no retries happen).
    let mock = server
        .mock("GET", "/pkg/versions.json")
        .with_status(404)
        .expect(2)
        .create();

    let (store, lists) = CountingStore::with_records(vec![bearer_record(
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
    let unauth_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(401)
        .expect(1)
        .create();
    let bearer_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer stored-token")
        .with_status(200)
        .expect(1)
        .create();

    let (store, lists) = CountingStore::with_records(vec![bearer_record(
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

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(lists.load(Ordering::SeqCst), 1);
    unauth_mock.assert();
    bearer_mock.assert();
}

#[test]
fn lazy_layer_env_credential_wins_without_store_read() {
    let mut server = mockito::Server::new();
    let unauth_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(401)
        .expect(1)
        .create();
    let env_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer env-token")
        .with_status(200)
        .expect(1)
        .create();

    let mut env_builder = StandardHTTPAuthenticationBuilder::new();
    env_builder.add_bearer_auth(format!("{}/**", server.url()), "env-token");
    let (store, lists) = CountingStore::with_records(vec![bearer_record(
        &[format!("{}/**", server.url())],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(env_builder.build().unwrap(), store);
    let runtime = runtime();

    let response = get(
        &runtime,
        &policy,
        &format!("{}/pkg/versions.json", server.url()),
    );

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(lists.load(Ordering::SeqCst), 0);
    unauth_mock.assert();
    env_mock.assert();
}

#[test]
fn lazy_layer_failed_env_credential_escalates_to_store() {
    let mut server = mockito::Server::new();
    let unauth_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(401)
        .expect(1)
        .create();
    let env_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer stale-env-token")
        .with_status(401)
        .expect(1)
        .create();
    let stored_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer stored-token")
        .with_status(200)
        .expect(1)
        .create();

    let mut env_builder = StandardHTTPAuthenticationBuilder::new();
    env_builder.add_bearer_auth(format!("{}/**", server.url()), "stale-env-token");
    let (store, lists) = CountingStore::with_records(vec![bearer_record(
        &[format!("{}/**", server.url())],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(env_builder.build().unwrap(), store);
    let runtime = runtime();

    let response = get(
        &runtime,
        &policy,
        &format!("{}/pkg/versions.json", server.url()),
    );

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(lists.load(Ordering::SeqCst), 1);
    unauth_mock.assert();
    env_mock.assert();
    stored_mock.assert();
}

#[test]
fn lazy_layer_overlapping_globs_of_one_record_retry_once() {
    let mut server = mockito::Server::new();
    let unauth_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(401)
        .expect(1)
        .create();
    // One login covering the URL with two patterns is not an ambiguity:
    // exactly one forced retry with its token.
    let bearer_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer stored-token")
        .with_status(200)
        .expect(1)
        .create();

    let (store, _lists) = CountingStore::with_records(vec![bearer_record(
        &[
            format!("{}/**", server.url()),
            format!("{}/pkg/*", server.url()),
        ],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();

    let response = get(
        &runtime,
        &policy,
        &format!("{}/pkg/versions.json", server.url()),
    );

    assert_eq!(response.status().as_u16(), 200);
    unauth_mock.assert();
    bearer_mock.assert();
}

#[test]
fn lazy_layer_ambiguous_distinct_records_try_all_in_order() {
    let mut server = mockito::Server::new();
    let unauth_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(401)
        .expect(1)
        .create();
    let rejected_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer broad-token")
        .with_status(401)
        .expect(1)
        .create();
    let accepted_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer narrow-token")
        .with_status(200)
        .expect(1)
        .create();

    let mut broad = bearer_record(&[format!("{}/**", server.url())], "broad-token");
    broad.key = "https://example.com/broad/".to_string();
    let mut narrow = bearer_record(&[format!("{}/pkg/*", server.url())], "narrow-token");
    narrow.key = "https://example.com/narrow/".to_string();
    let (store, _lists) = CountingStore::with_records(vec![broad, narrow]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();

    let response = get(
        &runtime,
        &policy,
        &format!("{}/pkg/versions.json", server.url()),
    );

    assert_eq!(response.status().as_u16(), 200);
    unauth_mock.assert();
    rejected_mock.assert();
    accepted_mock.assert();
}

#[test]
fn lazy_layer_invalid_stored_glob_skips_only_that_pattern() {
    let mut server = mockito::Server::new();
    let unauth_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(401)
        .expect(1)
        .create();
    let bearer_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer stored-token")
        .with_status(200)
        .expect(1)
        .create();

    let (store, _lists) = CountingStore::with_records(vec![bearer_record(
        &["[invalid".to_string(), format!("{}/**", server.url())],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();

    let response = get(
        &runtime,
        &policy,
        &format!("{}/pkg/versions.json", server.url()),
    );

    assert_eq!(response.status().as_u16(), 200);
    unauth_mock.assert();
    bearer_mock.assert();
}

#[test]
fn lazy_layer_store_errors_degrade_to_no_credentials() {
    for absent in [true, false] {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/pkg/versions.json")
            .with_status(404)
            .expect(2)
            .create();

        let (store, lists) = FailingStore::new(absent);
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
    let mock = server
        .mock("GET", "/pkg/versions.json")
        .with_status(404)
        .expect(1)
        .create();

    let policy: CredentialStoreAuthentication<_, InMemoryCredentialStore> =
        CredentialStoreAuthentication::without_store(empty_env_policy());
    let runtime = runtime();

    let response = get(
        &runtime,
        &policy,
        &format!("{}/pkg/versions.json", server.url()),
    );

    assert_eq!(response.status().as_u16(), 404);
    mock.assert();
}

#[test]
fn stored_bearer_map_blocking_shares_the_request_path_cache() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/pkg/versions.json")
        .with_status(404)
        .expect(1)
        .create();

    let (store, lists) = CountingStore::with_records(vec![bearer_record(
        &["https://other.example.com/**".to_string()],
        "stored-token",
    )]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();

    // Publish-style synchronous read first...
    let map = policy.stored_bearer_map_blocking();
    assert!(matches!(
        map.lookup("https://other.example.com/upload"),
        crate::auth::GlobMapResult::Found(_, _)
    ));
    // ...then a request-path escalation reuses the same cache.
    let response = get(
        &runtime,
        &policy,
        &format!("{}/pkg/versions.json", server.url()),
    );

    assert_eq!(response.status().as_u16(), 404);
    assert_eq!(lists.load(Ordering::SeqCst), 1);
    mock.assert();
}

// Reactive expiry hint (design/credential-storage.md section 9).

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
    assert!(
        message.contains("re-run `sysand auth login https://example.com/`"),
        "message: {message}"
    );
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
/// observe the per-record expiry-warned flag after driving requests.
fn stored_bearer_for<'a, Inner, S>(
    policy: &'a CredentialStoreAuthentication<Inner, S>,
    url: &str,
) -> &'a StoredBearerAuth
where
    S: CredentialStore + Send + Sync + 'static,
{
    match policy.stored_bearer_map_blocking().lookup(url) {
        crate::auth::GlobMapResult::Found(_, bearer) => bearer,
        other => panic!("expected a stored bearer for `{url}`, got {other:?}"),
    }
}

#[test]
fn lazy_layer_warns_once_for_an_expired_record_that_keeps_failing() {
    let mut server = mockito::Server::new();
    // GitLab-style host: bad auth answers 404, not 401 (the hint must
    // fire on any 4xx). Two full escalations; the flag must latch after
    // the first, so the hint is emitted at most once per process.
    let unauth_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(404)
        .expect(2)
        .create();
    let forced_mock = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer stored-token")
        .with_status(404)
        .expect(2)
        .create();

    let mut record = bearer_record(&[format!("{}/**", server.url())], "stored-token");
    record.expires_at = Some(Utc::now() - Duration::days(1));
    let (store, _lists) = CountingStore::with_records(vec![record]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();
    let url = format!("{}/pkg/versions.json", server.url());

    let first = get(&runtime, &policy, &url);
    // The hint was emitted (and latched) during the first escalation, so
    // the second identical failure cannot repeat it: `take_expiry_warning`
    // returns `None` once the flag is set (covered by the unit test above).
    assert!(stored_bearer_for(&policy, &url).expiry_warning_emitted());
    let second = get(&runtime, &policy, &url);

    assert_eq!(first.status().as_u16(), 404);
    assert_eq!(second.status().as_u16(), 404);
    assert!(stored_bearer_for(&policy, &url).expiry_warning_emitted());
    unauth_mock.assert();
    forced_mock.assert();
}

#[test]
fn lazy_layer_does_not_warn_for_an_unexpired_record_or_a_successful_retry() {
    // Case 1: unexpired record, failing retry: no hint.
    let mut server = mockito::Server::new();
    let _unauth = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(401)
        .create();
    let _forced = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer stored-token")
        .with_status(404)
        .create();
    let mut record = bearer_record(&[format!("{}/**", server.url())], "stored-token");
    record.expires_at = Some(Utc::now() + Duration::days(1));
    let (store, _lists) = CountingStore::with_records(vec![record]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let runtime = runtime();
    let url = format!("{}/pkg/versions.json", server.url());
    get(&runtime, &policy, &url);
    assert!(!stored_bearer_for(&policy, &url).expiry_warning_emitted());

    // Case 2: expired record, but the forced retry succeeds: no hint
    // (the credential demonstrably still works).
    let mut server = mockito::Server::new();
    let _unauth = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(401)
        .create();
    let _forced = server
        .mock("GET", "/pkg/versions.json")
        .match_header("authorization", "Bearer stored-token")
        .with_status(200)
        .create();
    let mut record = bearer_record(&[format!("{}/**", server.url())], "stored-token");
    record.expires_at = Some(Utc::now() - Duration::days(1));
    let (store, _lists) = CountingStore::with_records(vec![record]);
    let policy = CredentialStoreAuthentication::new(empty_env_policy(), store);
    let url = format!("{}/pkg/versions.json", server.url());
    let response = get(&runtime, &policy, &url);
    assert_eq!(response.status().as_u16(), 200);
    assert!(!stored_bearer_for(&policy, &url).expiry_warning_emitted());
}
