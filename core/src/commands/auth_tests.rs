// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use chrono::{TimeZone, Utc};

use super::{
    AuthCommandError, EnvCredentialEntry, StoredCredentialsStatus, assemble_auth_status,
    do_auth_logout, do_auth_status, validated_index_key,
};
use crate::credential_store::{
    CredentialRecord, CredentialScheme, CredentialStore, CredentialStoreError,
    InMemoryCredentialStore,
};

fn record(key: &str, secret: &str) -> CredentialRecord {
    CredentialRecord {
        key: key.to_string(),
        globs: vec![format!("{key}**")],
        scheme: CredentialScheme::Bearer,
        secret: secret.to_string(),
        expires_at: None,
        subject: None,
        token_name: None,
        token_prefix: None,
        extra: serde_json::Map::new(),
    }
}

fn store_with(records: &[CredentialRecord]) -> InMemoryCredentialStore {
    let mut store = InMemoryCredentialStore::new();
    for record in records {
        store.upsert(record.clone()).unwrap();
    }
    store
}

fn env_entry(label: &str, pattern: &str) -> EnvCredentialEntry {
    EnvCredentialEntry {
        label: label.to_string(),
        pattern: pattern.to_string(),
    }
}

// do_auth_logout

#[test]
fn logout_removes_only_the_matching_record() {
    let keep = record("https://other.example/", "tok-keep");
    let mut store = store_with(&[record("https://example.com/idx/", "tok-gone"), keep.clone()]);

    let key = do_auth_logout(&mut store, "https://example.com/idx/").unwrap();

    assert_eq!(key, "https://example.com/idx/");
    assert_eq!(store.list().unwrap(), vec![keep]);
}

#[test]
fn logout_normalizes_url_spellings_to_the_stored_key() {
    // Uppercase host, no trailing slash, default port: all the same key.
    let mut store = store_with(&[record("https://example.com/idx/", "tok")]);

    let key = do_auth_logout(&mut store, "HTTPS://Example.COM:443/idx").unwrap();

    assert_eq!(key, "https://example.com/idx/");
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn logout_of_missing_credential_errors() {
    let mut store = store_with(&[record("https://example.com/idx/", "tok")]);

    let err = do_auth_logout(&mut store, "https://absent.example/").unwrap_err();

    assert!(matches!(
        &err,
        AuthCommandError::NoStoredCredential { index } if index == "https://absent.example/"
    ));
    assert_eq!(
        err.to_string(),
        "no stored credential for `https://absent.example/`"
    );
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn logout_of_non_http_url_errors_without_touching_the_store() {
    let mut store = store_with(&[record("https://example.com/idx/", "tok")]);

    let err = do_auth_logout(&mut store, "file:///srv/index").unwrap_err();

    assert!(matches!(&err, AuthCommandError::NotHttpIndex { .. }));
    assert_eq!(
        err.to_string(),
        "`file:///srv/index`: not an HTTP(S) index; nothing to authenticate to"
    );
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn logout_of_unparseable_url_errors() {
    let mut store = InMemoryCredentialStore::new();

    let err = do_auth_logout(&mut store, "not a url").unwrap_err();

    assert!(matches!(&err, AuthCommandError::InvalidIndexUrl(_)));
}

// do_auth_status / assemble_auth_status

#[test]
fn status_lists_stored_records_and_env_entries() {
    let mut expiring = record("https://example.com/idx/", "tok-1");
    let expiry = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    expiring.expires_at = Some(expiry);
    expiring.globs = vec![
        "https://example.com/idx/**".to_string(),
        "https://api.example.com/**".to_string(),
    ];
    let store = store_with(&[expiring, record("https://other.example/", "tok-2")]);
    let env = vec![env_entry("SYSAND_CRED_CI", "https://ci.example/**")];

    let status = do_auth_status(&store, env.clone()).unwrap();

    assert_eq!(status.env, env);
    let StoredCredentialsStatus::Available(stored) = status.stored else {
        panic!("expected stored credentials to be available");
    };
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].key, "https://example.com/idx/");
    assert_eq!(
        stored[0].globs,
        vec![
            "https://example.com/idx/**".to_string(),
            "https://api.example.com/**".to_string(),
        ]
    );
    assert_eq!(stored[0].expires_at, Some(expiry));
    assert_eq!(stored[1].key, "https://other.example/");
    assert_eq!(stored[1].expires_at, None);
    assert!(!stored[1].expired);
}

#[test]
fn status_marks_expiry_against_the_given_clock() {
    let mut record = record("https://example.com/idx/", "tok");
    let expiry = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    record.expires_at = Some(expiry);

    let before = assemble_auth_status(
        vec![record.clone()],
        vec![],
        Utc.with_ymd_and_hms(2026, 8, 31, 0, 0, 0).unwrap(),
    );
    let after = assemble_auth_status(
        vec![record],
        vec![],
        Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap(),
    );

    let StoredCredentialsStatus::Available(before) = before.stored else {
        panic!("expected stored credentials");
    };
    let StoredCredentialsStatus::Available(after) = after.stored else {
        panic!("expected stored credentials");
    };
    assert!(!before[0].expired);
    assert!(after[0].expired);
}

#[test]
fn status_reports_env_entries_shadowing_a_stored_key() {
    let records = vec![
        record("https://example.com/idx/", "tok-1"),
        record("https://other.example/", "tok-2"),
    ];
    let env = vec![
        env_entry("SYSAND_CRED_TEAM", "https://example.com/**"),
        env_entry("SYSAND_CRED_ELSEWHERE", "https://unrelated.example/**"),
        // Invalid pattern: cannot shadow, must be skipped, not panic.
        env_entry("SYSAND_CRED_BROKEN", "https://example.com/[invalid"),
    ];

    let status = assemble_auth_status(records, env, Utc::now());

    let StoredCredentialsStatus::Available(stored) = status.stored else {
        panic!("expected stored credentials");
    };
    assert_eq!(stored[0].shadowed_by, vec!["SYSAND_CRED_TEAM".to_string()]);
    assert!(stored[1].shadowed_by.is_empty());
}

/// A store whose reads fail with a configurable error, for the error
/// taxonomy paths.
struct FailingStore(fn() -> CredentialStoreError);

impl CredentialStore for FailingStore {
    fn list(&self) -> Result<Vec<CredentialRecord>, CredentialStoreError> {
        Err((self.0)())
    }

    fn upsert(&mut self, _record: CredentialRecord) -> Result<(), CredentialStoreError> {
        Err((self.0)())
    }

    fn remove(&mut self, _key: &str) -> Result<bool, CredentialStoreError> {
        Err((self.0)())
    }
}

#[test]
fn status_degrades_to_env_only_when_the_backend_is_absent() {
    let store = FailingStore(|| CredentialStoreError::BackendAbsent {
        source: "no secret service".into(),
    });
    let env = vec![env_entry("SYSAND_CRED_CI", "https://ci.example/**")];

    let status = do_auth_status(&store, env.clone()).unwrap();

    assert_eq!(status.env, env);
    assert!(matches!(
        status.stored,
        StoredCredentialsStatus::BackendUnavailable { reason } if reason == "no secret service"
    ));
}

#[test]
fn status_surfaces_a_denied_backend_as_an_error() {
    let store = FailingStore(|| CredentialStoreError::BackendDenied {
        source: "collection is locked".into(),
    });

    let err = do_auth_status(&store, vec![]).unwrap_err();

    assert!(matches!(
        err,
        AuthCommandError::Store(CredentialStoreError::BackendDenied { .. })
    ));
}

// do_auth_login

mod login {
    use std::sync::Arc;

    use globset::{GlobBuilder, GlobSetBuilder};

    use super::super::{AuthLoginNotice, AuthLoginOutcome, do_auth_login};
    use super::*;
    use crate::{index_location::IndexLocation, resolve::net_utils::create_reqwest_client};

    fn make_runtime() -> Arc<tokio::runtime::Runtime> {
        Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        )
    }

    /// Run `do_auth_login` with the given `validation` argument against a
    /// real client/runtime, collecting the notices in order.
    fn run_login_with<S: CredentialStore>(
        store: &mut S,
        index_url: &str,
        secret: &str,
        validation: Option<bool>,
    ) -> (
        Result<AuthLoginOutcome, AuthCommandError>,
        Vec<AuthLoginNotice>,
    ) {
        let client = create_reqwest_client().unwrap();
        let mut notices = Vec::new();
        let outcome = do_auth_login(
            store,
            index_url,
            secret.to_string(),
            validation,
            &client,
            make_runtime(),
            |notice| notices.push(notice),
        );
        (outcome, notices)
    }

    /// Run `do_auth_login` with validation disabled: these tests cover
    /// glob derivation and store behavior, which `--validation false`
    /// preserves exactly, without needing probe mocks.
    fn run_login<S: CredentialStore>(
        store: &mut S,
        index_url: &str,
        secret: &str,
    ) -> (
        Result<AuthLoginOutcome, AuthCommandError>,
        Vec<AuthLoginNotice>,
    ) {
        run_login_with(store, index_url, secret, Some(false))
    }

    /// Compile globs exactly the way runtime matching does
    /// (`GlobMapBuilder`: `literal_separator(true)`).
    fn matcher(globs: &[String]) -> globset::GlobSet {
        let mut builder = GlobSetBuilder::new();
        for glob in globs {
            builder.add(
                GlobBuilder::new(glob)
                    .literal_separator(true)
                    .build()
                    .unwrap_or_else(|err| panic!("glob `{glob}` must compile: {err}")),
            );
        }
        builder.build().unwrap()
    }

    fn stored(outcome: Result<AuthLoginOutcome, AuthCommandError>) -> (String, Vec<String>) {
        let (key, globs, _) = stored_validated(outcome);
        (key, globs)
    }

    fn stored_validated(
        outcome: Result<AuthLoginOutcome, AuthCommandError>,
    ) -> (String, Vec<String>, Vec<super::super::ProbeSurface>) {
        match outcome.unwrap() {
            AuthLoginOutcome::Stored {
                key,
                globs,
                validated,
            } => (key, globs, validated),
            other => panic!("expected Stored, got {other:?}"),
        }
    }

    /// The section 8 coverage guarantee: the discovery-document URL, the
    /// `index.json` URL, and the upload URL each match the derived set.
    fn assert_surface_coverage(
        globs: &[String],
        discovery_root: &str,
        index_root: &str,
        api_root: &str,
    ) {
        let set = matcher(globs);
        for url in [
            format!("{discovery_root}sysand-index-config.json"),
            format!("{index_root}index.json"),
            format!("{api_root}v1/upload"),
        ] {
            assert!(set.is_match(&url), "`{url}` must match globs {globs:?}");
        }
    }

    fn config_mock(server: &mut mockito::Server, body: String) -> mockito::Mock {
        server
            .mock("GET", "/sysand-index-config.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create()
    }

    #[test]
    fn login_without_discovery_document_derives_a_single_glob() {
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _mock = server
            .mock("GET", "/sysand-index-config.json")
            .with_status(404)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login(&mut store, &server.url(), "tok");

        let (key, globs) = stored(outcome);
        assert_eq!(key, root);
        assert_eq!(globs, vec![format!("{}**", globset::escape(&root))]);
        assert!(notices.is_empty(), "unexpected notices: {notices:?}");
        // Flat topology: every surface lives under the discovery root.
        assert_surface_coverage(&globs, &root, &root, &root);
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    #[test]
    fn login_with_nested_api_root_keeps_one_glob() {
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let api_root = format!("{root}api/");
        let _mock = config_mock(&mut server, format!(r#"{{"api_root": "{api_root}"}}"#));
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login(&mut store, &root, "tok");

        let (_, globs) = stored(outcome);
        assert_eq!(globs.len(), 1, "Case A must not add a glob: {globs:?}");
        assert!(notices.is_empty());
        assert_surface_coverage(&globs, &root, &root, &api_root);
    }

    #[test]
    fn login_with_disjoint_api_root_adds_a_second_glob() {
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let api_root = "https://api.example.com/base/";
        let _mock = config_mock(&mut server, format!(r#"{{"api_root": "{api_root}"}}"#));
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login(&mut store, &root, "tok");

        let (_, globs) = stored(outcome);
        assert_eq!(
            globs,
            vec![
                format!("{}**", globset::escape(&root)),
                format!("{}**", globset::escape(api_root)),
            ]
        );
        assert_surface_coverage(&globs, &root, &root, api_root);
        // Non-overlapping: the api glob stays host-scoped.
        assert!(!matcher(&globs[1..]).is_match(format!("{root}index.json")));
    }

    #[test]
    fn login_with_divergent_index_and_api_roots_adds_three_globs() {
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let index_root = "https://files.example.com/idx/";
        let api_root = "https://api.example.com/";
        let _mock = config_mock(
            &mut server,
            format!(r#"{{"index_root": "{index_root}", "api_root": "{api_root}"}}"#),
        );
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login(&mut store, &root, "tok");

        let (_, globs) = stored(outcome);
        assert_eq!(globs.len(), 3, "Case B twice over: {globs:?}");
        assert_surface_coverage(&globs, &root, index_root, api_root);
    }

    #[test]
    fn login_with_templated_index_root_anchors_before_the_placeholder() {
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let template = "https://files.example.com/repo/{path}/raw?ref=main";
        let api_root = "https://api.example.com/";
        let _mock = config_mock(
            &mut server,
            format!(r#"{{"index_root": "{template}", "api_root": "{api_root}"}}"#),
        );
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login(&mut store, &root, "tok");

        let (_, globs) = stored(outcome);
        assert!(notices.is_empty(), "unexpected notices: {notices:?}");
        assert_eq!(
            globs[1],
            format!("{}**", globset::escape("https://files.example.com/repo/"))
        );
        // The templated index.json URL (placeholder expansion) matches.
        let index_json = IndexLocation::parse(template).unwrap().resolve([
            "some-publisher",
            "some.name",
            "index.json",
        ]);
        assert!(matcher(&globs).is_match(index_json.as_str()));
        assert_surface_coverage(&globs, &root, "https://files.example.com/repo/", api_root);
    }

    #[test]
    fn login_skips_an_unanchorable_template_index_root() {
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        // Placeholder directly in the query of a path-less URL: the only
        // `/` in the literal prefix is the one in `://`, so anchoring
        // would produce a glob matching every https URL.
        let template = "https://files.example.com?f={path}";
        let _mock = config_mock(&mut server, format!(r#"{{"index_root": "{template}"}}"#));
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login(&mut store, &root, "tok");

        let (_, globs) = stored(outcome);
        assert!(
            notices
                .iter()
                .any(|n| matches!(n, AuthLoginNotice::TemplateIndexRootSkipped { .. })),
            "expected a skip notice, got {notices:?}"
        );
        assert!(
            !matcher(&globs).is_match("https://attacker.example/x"),
            "no derived glob may cover other hosts: {globs:?}"
        );
    }

    #[test]
    fn login_with_unauthorized_discovery_falls_back_with_a_notice() {
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _mock = server
            .mock("GET", "/sysand-index-config.json")
            .with_status(401)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login(&mut store, &root, "tok");

        let (_, globs) = stored(outcome);
        assert_eq!(globs, vec![format!("{}**", globset::escape(&root))]);
        assert!(
            notices.iter().any(|n| matches!(
                n,
                AuthLoginNotice::DiscoveryUnreachable { error } if error.contains("401")
            )),
            "expected a 401 discovery notice, got {notices:?}"
        );
    }

    #[test]
    fn login_with_unreachable_ipv6_literal_derives_an_escaped_glob() {
        // Port 1 answers nothing, so discovery is unreachable and the
        // URL-derived fallback glob is used. The IPv6 literal would read
        // as a globset character class without escaping.
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login(&mut store, "https://[::1]:1", "tok");

        let (key, globs) = stored(outcome);
        assert_eq!(key, "https://[::1]:1/");
        assert!(
            notices
                .iter()
                .any(|n| matches!(n, AuthLoginNotice::DiscoveryUnreachable { .. }))
        );
        let set = matcher(&globs);
        assert!(set.is_match("https://[::1]:1/index.json"));
        assert!(set.is_match("https://[::1]:1/sysand-index-config.json"));
        assert!(!set.is_match("https://[x1]:1/index.json"));
    }

    #[test]
    fn login_over_an_existing_key_notifies_replacement_and_overwrites() {
        let mut store = InMemoryCredentialStore::new();
        let (first, first_notices) = run_login(&mut store, "http://127.0.0.1:1", "old-tok");
        stored(first);
        assert!(
            !first_notices
                .iter()
                .any(|n| matches!(n, AuthLoginNotice::ReplacingExisting { .. }))
        );

        let (second, second_notices) = run_login(&mut store, "http://127.0.0.1:1/", "new-tok");

        stored(second);
        assert!(second_notices.iter().any(|n| matches!(
            n,
            AuthLoginNotice::ReplacingExisting { key } if key == "http://127.0.0.1:1/"
        )));
        let records = store.list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].secret, "new-tok");
    }

    #[test]
    fn login_reports_an_absent_backend_with_the_derived_globs() {
        let mut store = FailingStore(|| CredentialStoreError::BackendAbsent {
            source: "no secret service".into(),
        });

        let (outcome, _) = run_login(&mut store, "http://127.0.0.1:1", "tok");

        match outcome.unwrap() {
            AuthLoginOutcome::BackendUnavailable { key, globs, reason } => {
                assert_eq!(key, "http://127.0.0.1:1/");
                assert_eq!(globs, vec![format!("{}**", globset::escape(&key))]);
                assert_eq!(reason, "no secret service");
            }
            other => panic!("expected BackendUnavailable, got {other:?}"),
        }
    }

    // Template targets (design/credential-storage.md section 4: templated
    // login targets are supported, anchored per section 8).

    /// The motivating GitLab-shaped target: the key is the template text
    /// itself (already canonical), with the anchor normalizing only the
    /// URL-parsed part of the literal prefix.
    #[test]
    fn template_key_is_the_template_with_a_normalized_anchor() {
        let gitlab =
            "https://gitlab.com/api/v4/projects/84113019/repository/files/{path}/raw?ref=index";

        assert_eq!(validated_index_key(gitlab).unwrap(), gitlab);
        // Scheme/host case and the default port normalize through the
        // anchor; the placeholder and suffix stay verbatim.
        let spelled =
            "HTTPS://GitLab.COM:443/api/v4/projects/84113019/repository/files/{path}/raw?ref=index";
        assert_eq!(validated_index_key(spelled).unwrap(), gitlab);
        // Idempotent: the key is its own key.
        assert_eq!(
            validated_index_key(&validated_index_key(spelled).unwrap()).unwrap(),
            gitlab
        );
    }

    #[test]
    fn template_key_rejects_an_unanchorable_template() {
        // Placeholder directly in the query of a path-less URL: the only
        // `/` in the literal prefix is the one in `://`.
        let err = validated_index_key("https://files.example.com?f={path}").unwrap_err();

        assert!(matches!(
            &err,
            AuthCommandError::TemplateWithoutAnchor { .. }
        ));
        let message = err.to_string();
        assert!(message.contains("SYSAND_CRED_"), "was: {message}");
    }

    #[test]
    fn template_key_keeps_the_non_http_rejection() {
        let err = validated_index_key("ftp://files.example.com/{path}").unwrap_err();

        assert!(matches!(&err, AuthCommandError::NotHttpIndex { .. }));
    }

    #[test]
    fn login_with_a_template_target_anchors_and_covers_expanded_urls() {
        let mut server = mockito::Server::new();
        let template = format!("{}/repo/files/{{path}}/raw?ref=index", server.url());
        let anchor = format!("{}/repo/files/", server.url());
        // No discovery document: the fetch goes through the template
        // (asserted indirectly: a missed mock would answer 501, which is
        // a DiscoveryUnreachable notice, and notices must stay empty).
        let _config = server
            .mock("GET", "/repo/files/sysand-index-config.json/raw")
            .match_query(mockito::Matcher::UrlEncoded("ref".into(), "index".into()))
            .with_status(404)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login(&mut store, &template, "tok");

        let (key, globs) = stored(outcome);
        assert_eq!(key, template);
        assert!(notices.is_empty(), "unexpected notices: {notices:?}");
        assert_eq!(globs, vec![format!("{}**", globset::escape(&anchor))]);
        // Section 8 coverage for a template without an api_root: the
        // template-resolved discovery URL, `index.json` URL, and an
        // encoded-`{path}` project-file URL all match the derived set.
        let location = IndexLocation::parse(&template).unwrap();
        let set = matcher(&globs);
        let kpar = location.resolve(["some-publisher", "some.name", "1.0.0", "project.kpar"]);
        for url in [
            location.resolve(["sysand-index-config.json"]),
            location.resolve(["index.json"]),
            kpar.clone(),
        ] {
            assert!(set.is_match(url.as_str()), "`{url}` must match {globs:?}");
        }
        // The `{path}` expansion really is one encoded segment.
        assert!(kpar.as_str().contains("some-publisher%2Fsome.name"));
        assert!(!set.is_match(format!("{}/elsewhere", server.url())));
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    #[test]
    fn login_and_logout_round_trip_a_template_key() {
        // Port 1 answers nothing: discovery is unreachable and the glob
        // set is the anchor-derived fallback.
        let template = "http://127.0.0.1:1/files/{path}/raw?ref=main";
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login(&mut store, template, "tok");

        let (key, globs) = stored(outcome);
        assert_eq!(key, template);
        assert_eq!(
            globs,
            vec![format!(
                "{}**",
                globset::escape("http://127.0.0.1:1/files/")
            )]
        );
        assert!(
            notices
                .iter()
                .any(|n| matches!(n, AuthLoginNotice::DiscoveryUnreachable { .. }))
        );
        // `status` reports the key in the exact form `logout` accepts.
        let status = do_auth_status(&store, vec![]).unwrap();
        let StoredCredentialsStatus::Available(stored) = status.stored else {
            panic!("expected stored credentials");
        };
        assert_eq!(stored[0].key, template);
        // A differently-spelled scheme normalizes to the same key.
        let removed =
            do_auth_logout(&mut store, "HTTP://127.0.0.1:1/files/{path}/raw?ref=main").unwrap();
        assert_eq!(removed, template);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn login_with_a_query_placeholder_template_anchors_at_the_path() {
        // Anchorable query-position placeholder: the anchor cuts back to
        // the last `/` of the literal prefix, shallower than the whole
        // prefix.
        let template = "http://127.0.0.1:1/repo/download?file={path}&ref=main";
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login(&mut store, template, "tok");

        let (key, globs) = stored(outcome);
        assert_eq!(key, template);
        assert_eq!(
            globs,
            vec![format!("{}**", globset::escape("http://127.0.0.1:1/repo/"))]
        );
        let expanded = IndexLocation::parse(template)
            .unwrap()
            .resolve(["index.json"]);
        assert!(matcher(&globs).is_match(expanded.as_str()));
        assert!(!matcher(&globs).is_match("http://127.0.0.1:1/other"));
    }

    #[test]
    fn login_with_a_path_raw_template_target_round_trips() {
        let template = "http://127.0.0.1:1/raw/{path_raw}?ref=main";
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login(&mut store, template, "tok");

        let (key, globs) = stored(outcome);
        assert_eq!(key, template);
        // `{path_raw}` keeps literal `/` separators; `**` after `/`
        // crosses them.
        let expanded =
            IndexLocation::parse(template)
                .unwrap()
                .resolve(["pub", "name", "versions.json"]);
        assert!(expanded.as_str().contains("/raw/pub/name/versions.json"));
        assert!(matcher(&globs).is_match(expanded.as_str()));
        assert_eq!(do_auth_logout(&mut store, template).unwrap(), template);
    }

    #[test]
    fn login_rejects_an_unanchorable_template_target() {
        let mut store = InMemoryCredentialStore::new();

        // Fails in key validation, before any network or store access.
        let (outcome, notices) = run_login(&mut store, "https://files.example.com?f={path}", "tok");

        assert!(matches!(
            outcome.unwrap_err(),
            AuthCommandError::TemplateWithoutAnchor { .. }
        ));
        assert!(notices.is_empty());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn logout_rejects_an_unanchorable_template_target() {
        let mut store = InMemoryCredentialStore::new();

        let err = do_auth_logout(&mut store, "https://files.example.com?f={path}").unwrap_err();

        assert!(matches!(
            err,
            AuthCommandError::TemplateWithoutAnchor { .. }
        ));
    }

    #[test]
    fn validated_login_on_a_private_template_index_validates_read() {
        // The GitLab reality check: unauthenticated GET answers 404 on a
        // private repo, the forced bearer retry 200s; that is the
        // accepted-read path. A template without a discovery document
        // has no api_root, so whoami is never probed.
        let mut server = mockito::Server::new();
        let template = format!("{}/repo/files/{{path}}/raw?ref=index", server.url());
        let ref_query = mockito::Matcher::UrlEncoded("ref".into(), "index".into());
        let _config = server
            .mock("GET", "/repo/files/sysand-index-config.json/raw")
            .match_query(ref_query.clone())
            .with_status(404)
            .create();
        let unauth = server
            .mock("GET", "/repo/files/index.json/raw")
            .match_query(ref_query.clone())
            .match_header("authorization", mockito::Matcher::Missing)
            .with_status(404)
            .expect(1)
            .create();
        let forced = server
            .mock("GET", "/repo/files/index.json/raw")
            .match_query(ref_query)
            .match_header("authorization", "Bearer tok")
            .with_status(200)
            .expect(1)
            .create();
        let whoami = server
            .mock("GET", mockito::Matcher::Regex("whoami".to_string()))
            .expect(0)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &template, "tok", None);

        let (key, _, validated) = stored_validated(outcome);
        assert_eq!(key, template);
        assert_eq!(validated, vec![ProbeSurface::Read]);
        assert!(notices.is_empty(), "unexpected notices: {notices:?}");
        unauth.assert();
        forced.assert();
        whoami.assert();
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    // Validation probes (design/credential-storage.md section 5).
    //
    // All tests follow the existing pattern: a sync `mockito::Server`
    // created before any `runtime.block_on` (never `#[tokio::test]`, which
    // would nest runtimes).

    use super::super::ProbeSurface;

    const WHOAMI_BODY: &str = r#"{"subject":{"type":"user","name":"alice"},
        "token":{"name":"laptop","prefix":"sysand_u_1a2b3c4d",
                 "expires_at":"2026-09-01T00:00:00Z"}}"#;

    fn no_discovery_mock(server: &mut mockito::Server) -> mockito::Mock {
        server
            .mock("GET", "/sysand-index-config.json")
            .with_status(404)
            .create()
    }

    /// Mock a private `index.json`: 401 without credentials,
    /// `forced_status` with exactly the expected bearer token.
    fn private_index_json(
        server: &mut mockito::Server,
        token: &str,
        forced_status: usize,
    ) -> (mockito::Mock, mockito::Mock) {
        let unauth = server
            .mock("GET", "/index.json")
            .match_header("authorization", mockito::Matcher::Missing)
            .with_status(401)
            .create();
        let forced = server
            .mock("GET", "/index.json")
            .match_header("authorization", format!("Bearer {token}").as_str())
            .with_status(forced_status)
            .create();
        (unauth, forced)
    }

    #[test]
    fn validated_login_on_public_index_without_api_stores_not_validated() {
        // S1: public read, no API. Nothing exercises the credential, and
        // the flat/defaulted api_root must never be phantom-probed.
        let mut server = mockito::Server::new();
        let _config = no_discovery_mock(&mut server);
        let _index = server.mock("GET", "/index.json").with_status(200).create();
        let whoami = server.mock("GET", "/v1/whoami").expect(0).create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert!(validated.is_empty(), "nothing was exercised: {validated:?}");
        assert!(notices.is_empty(), "unexpected notices: {notices:?}");
        whoami.assert();
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    #[test]
    fn validated_login_does_not_probe_a_defaulted_api_root() {
        // A discovery document without `api_root` defaults the API to the
        // discovery root at runtime, but that default is not an
        // advertisement: no whoami probe.
        let mut server = mockito::Server::new();
        let _config = config_mock(&mut server, "{}".to_string());
        let _index = server.mock("GET", "/index.json").with_status(200).create();
        let whoami = server.mock("GET", "/v1/whoami").expect(0).create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert!(validated.is_empty());
        whoami.assert();
    }

    #[test]
    fn validated_login_on_private_read_index_validates_read() {
        // S2: private static index. The read baseline 401s, the forced
        // retry succeeds: validated (read).
        let mut server = mockito::Server::new();
        let _config = no_discovery_mock(&mut server);
        let (unauth, forced) = private_index_json(&mut server, "tok", 200);
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert_eq!(validated, vec![ProbeSurface::Read]);
        assert!(notices.is_empty(), "unexpected notices: {notices:?}");
        unauth.assert();
        forced.assert();
        let record = &store.list().unwrap()[0];
        assert_eq!(record.secret, "tok");
        assert_eq!(record.subject, None);
        assert_eq!(record.expires_at, None);
    }

    #[test]
    fn validated_login_probes_read_when_discovery_is_private() {
        // A fully private index rejects even the discovery fetch; the
        // read probe falls back to the URL-derived `index.json` so the
        // credential still gets exercised.
        let mut server = mockito::Server::new();
        let _config = server
            .mock("GET", "/sysand-index-config.json")
            .with_status(401)
            .create();
        let (_unauth, _forced) = private_index_json(&mut server, "tok", 200);
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert_eq!(validated, vec![ProbeSurface::Read]);
        assert!(
            notices
                .iter()
                .any(|n| matches!(n, AuthLoginNotice::DiscoveryUnreachable { .. }))
        );
    }

    #[test]
    fn validated_login_refuses_when_the_only_exercised_surface_rejects() {
        // The refusal rule: read rejected, nothing else exercised. An
        // existing login for the same key must survive untouched, and no
        // replacement may be announced.
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _config = no_discovery_mock(&mut server);
        let _index = server
            .mock("GET", "/index.json")
            .with_status(401)
            .expect(2)
            .create();
        let mut store = store_with(&[record(&root, "old-tok")]);

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "bad-tok", None);

        let err = outcome.unwrap_err();
        assert!(matches!(
            &err,
            AuthCommandError::ValidationRejected {
                index,
                rejected,
                basic_challenge: false,
            } if index == &root && rejected == &vec![ProbeSurface::Read]
        ));
        assert!(err.to_string().contains("nothing was stored"));
        assert!(
            !notices
                .iter()
                .any(|n| matches!(n, AuthLoginNotice::ReplacingExisting { .. })),
            "a refused login must not announce a replacement: {notices:?}"
        );
        assert_eq!(store.list().unwrap()[0].secret, "old-tok");
    }

    #[test]
    fn validated_login_validates_an_advertised_api_and_persists_identity() {
        // S3: public read, advertised API. whoami 200 validates (api) and
        // its identity fields are persisted on the record.
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _config = config_mock(&mut server, format!(r#"{{"api_root": "{root}api/"}}"#));
        let _index = server.mock("GET", "/index.json").with_status(200).create();
        let whoami = server
            .mock("GET", "/api/v1/whoami")
            .match_header("authorization", "Bearer tok")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(WHOAMI_BODY)
            .expect(1)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert_eq!(validated, vec![ProbeSurface::Api]);
        assert!(notices.is_empty(), "unexpected notices: {notices:?}");
        whoami.assert();
        let record = &store.list().unwrap()[0];
        assert_eq!(
            record.subject,
            Some(crate::credential_store::CredentialSubject {
                kind: "user".to_string(),
                name: "alice".to_string(),
            })
        );
        assert_eq!(record.token_name.as_deref(), Some("laptop"));
        assert_eq!(record.token_prefix.as_deref(), Some("sysand_u_1a2b3c4d"));
        assert_eq!(
            record.expires_at,
            Some(Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap())
        );
    }

    #[test]
    fn validated_login_refuses_when_whoami_rejects_on_a_public_index() {
        // Public read never tests the token, so whoami is the only real
        // test; a 401 there refuses, keeping the publish flow protected.
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _config = config_mock(&mut server, format!(r#"{{"api_root": "{root}api/"}}"#));
        let _index = server.mock("GET", "/index.json").with_status(200).create();
        let _whoami = server
            .mock("GET", "/api/v1/whoami")
            .with_status(401)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login_with(&mut store, &server.url(), "tok", None);

        assert!(matches!(
            outcome.unwrap_err(),
            AuthCommandError::ValidationRejected { rejected, .. }
                if rejected == vec![ProbeSurface::Api]
        ));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn validated_login_stores_with_a_warning_when_read_accepts_and_api_rejects() {
        // Private index, read works, API rejects: store with an
        // "API access failed" warning (the token still reads).
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _config = config_mock(&mut server, format!(r#"{{"api_root": "{root}api/"}}"#));
        let (_unauth, _forced) = private_index_json(&mut server, "tok", 200);
        let _whoami = server
            .mock("GET", "/api/v1/whoami")
            .with_status(401)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert_eq!(validated, vec![ProbeSurface::Read]);
        assert!(
            notices.iter().any(|n| matches!(
                n,
                AuthLoginNotice::SurfaceRejected {
                    surface: ProbeSurface::Api,
                    basic_challenge: false,
                }
            )),
            "expected an api rejection notice, got {notices:?}"
        );
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    #[test]
    fn validated_login_validates_both_surfaces() {
        // S4: private read accepts and whoami accepts: validated
        // (read, api), in probe order.
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _config = config_mock(&mut server, format!(r#"{{"api_root": "{root}api/"}}"#));
        let (_unauth, _forced) = private_index_json(&mut server, "tok", 200);
        let _whoami = server
            .mock("GET", "/api/v1/whoami")
            .with_status(200)
            .with_body(WHOAMI_BODY)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert_eq!(validated, vec![ProbeSurface::Read, ProbeSurface::Api]);
        assert!(store.list().unwrap()[0].subject.is_some());
    }

    #[test]
    fn validated_login_stores_not_validated_when_probes_are_unreachable() {
        // 5xx is not a verdict: the surface was not tested, and with
        // nothing else exercised the login stores as not validated.
        let mut server = mockito::Server::new();
        let _config = no_discovery_mock(&mut server);
        let _index = server.mock("GET", "/index.json").with_status(500).create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert!(validated.is_empty());
        assert!(
            notices.iter().any(|n| matches!(
                n,
                AuthLoginNotice::ProbeUnreachable {
                    surface: ProbeSurface::Read,
                    ..
                }
            )),
            "expected an unreachable notice, got {notices:?}"
        );
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    #[test]
    fn validated_login_treats_a_redirected_probe_as_not_tested() {
        // Probes never follow redirects; the verdict would come from a
        // different URL than the surface nominally probed.
        let target = "https://elsewhere.example/idx/index.json";
        let mut server = mockito::Server::new();
        let _config = no_discovery_mock(&mut server);
        let _index = server
            .mock("GET", "/index.json")
            .with_status(302)
            .with_header("location", target)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert!(validated.is_empty());
        assert!(
            notices.iter().any(|n| matches!(
                n,
                AuthLoginNotice::ProbeRedirected {
                    surface: ProbeSurface::Read,
                    target: t,
                } if t == target
            )),
            "expected a redirect notice naming `{target}`, got {notices:?}"
        );
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    #[test]
    fn validated_login_refusal_names_basic_authentication() {
        // A basic-auth index must not dead-end: the refusal routes the
        // user to the `SYSAND_CRED_*` basic credentials.
        let mut server = mockito::Server::new();
        let _config = no_discovery_mock(&mut server);
        let _index = server
            .mock("GET", "/index.json")
            .with_status(401)
            .with_header("www-authenticate", r#"Basic realm="idx""#)
            .expect(2)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login_with(&mut store, &server.url(), "tok", None);

        let err = outcome.unwrap_err();
        assert!(matches!(
            &err,
            AuthCommandError::ValidationRejected {
                basic_challenge: true,
                ..
            }
        ));
        let message = err.to_string();
        assert!(message.contains("username/password"), "was: {message}");
        assert!(
            message.contains("SYSAND_CRED_<X>_BASIC_USER"),
            "was: {message}"
        );
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn validated_login_ignores_a_bearer_challenge_mentioning_basic() {
        // Scheme detection must not be a substring check: a `Bearer`
        // challenge whose realm mentions Basic is not a basic challenge.
        let mut server = mockito::Server::new();
        let _config = no_discovery_mock(&mut server);
        let _index = server
            .mock("GET", "/index.json")
            .with_status(401)
            .with_header("www-authenticate", r#"Bearer realm="Basic migration""#)
            .expect(2)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login_with(&mut store, &server.url(), "tok", None);

        assert!(matches!(
            outcome.unwrap_err(),
            AuthCommandError::ValidationRejected {
                basic_challenge: false,
                ..
            }
        ));
    }

    #[test]
    fn validated_login_accepts_whoami_with_an_unparseable_body() {
        // The 200 status is the acceptance verdict; a body this client
        // cannot parse only loses the identity fields.
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _config = config_mock(&mut server, format!(r#"{{"api_root": "{root}api/"}}"#));
        let _index = server.mock("GET", "/index.json").with_status(200).create();
        let _whoami = server
            .mock("GET", "/api/v1/whoami")
            .with_status(200)
            .with_body("not json")
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, _) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert_eq!(validated, vec![ProbeSurface::Api]);
        let record = &store.list().unwrap()[0];
        assert_eq!(record.subject, None);
        assert_eq!(record.token_name, None);
        assert_eq!(record.token_prefix, None);
        assert_eq!(record.expires_at, None);
    }

    // 429 carve-out (design/credential-storage.md section 5): a 429 is
    // never a verdict, so rate limiting can never refuse a credential.

    #[test]
    fn validated_login_treats_a_rate_limited_baseline_as_not_tested() {
        // A 429 baseline sends no forced retry (it would spend more of
        // the rate budget and prove nothing): stored, not validated.
        let mut server = mockito::Server::new();
        let _config = no_discovery_mock(&mut server);
        let baseline = server
            .mock("GET", "/index.json")
            .match_header("authorization", mockito::Matcher::Missing)
            .with_status(429)
            .expect(1)
            .create();
        let forced = server
            .mock("GET", "/index.json")
            .match_header("authorization", "Bearer tok")
            .expect(0)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert!(validated.is_empty(), "429 must not validate: {validated:?}");
        assert_eq!(
            notices,
            vec![AuthLoginNotice::ProbeRateLimited {
                surface: ProbeSurface::Read,
            }]
        );
        baseline.assert();
        forced.assert();
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    #[test]
    fn validated_login_stores_when_the_forced_retry_is_rate_limited() {
        // Formerly false-refusing sequence: baseline 401, forced 429
        // counted as rejected and refused a possibly valid token. Now the
        // surface is not tested and the credential is stored with a
        // rate-limited warning.
        let mut server = mockito::Server::new();
        let _config = no_discovery_mock(&mut server);
        let (unauth, forced) = private_index_json(&mut server, "tok", 429);
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert!(validated.is_empty(), "429 must not validate: {validated:?}");
        assert_eq!(
            notices,
            vec![AuthLoginNotice::ProbeRateLimited {
                surface: ProbeSurface::Read,
            }]
        );
        unauth.assert();
        forced.assert();
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    #[test]
    fn validated_login_treats_a_rate_limited_whoami_as_not_tested() {
        // Public read, advertised API, whoami rate limited: nothing was
        // tested, so the credential stores as "not validated" instead of
        // being refused by the throttle.
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _config = config_mock(&mut server, format!(r#"{{"api_root": "{root}api/"}}"#));
        let _index = server.mock("GET", "/index.json").with_status(200).create();
        let whoami = server
            .mock("GET", "/api/v1/whoami")
            .with_status(429)
            .expect(1)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert!(validated.is_empty(), "429 must not validate: {validated:?}");
        assert_eq!(
            notices,
            vec![AuthLoginNotice::ProbeRateLimited {
                surface: ProbeSurface::Api,
            }]
        );
        whoami.assert();
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }

    #[test]
    fn validated_login_validates_api_when_read_is_rate_limited() {
        // A throttled read surface must not mask a working API probe: the
        // stored claim is scoped to what actually accepted.
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let _config = config_mock(&mut server, format!(r#"{{"api_root": "{root}api/"}}"#));
        let _index = server.mock("GET", "/index.json").with_status(429).create();
        let _whoami = server
            .mock("GET", "/api/v1/whoami")
            .match_header("authorization", "Bearer tok")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(WHOAMI_BODY)
            .create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", None);

        let (_, _, validated) = stored_validated(outcome);
        assert_eq!(validated, vec![ProbeSurface::Api]);
        assert_eq!(
            notices,
            vec![AuthLoginNotice::ProbeRateLimited {
                surface: ProbeSurface::Read,
            }]
        );
    }

    #[test]
    fn login_without_validation_sends_no_probe_requests() {
        // `--validation false` preserves the pre-validation behavior
        // exactly: discovery is still fetched for glob scoping, but no
        // probe request is ever made.
        let mut server = mockito::Server::new();
        let root = format!("{}/", server.url());
        let config = server
            .mock("GET", "/sysand-index-config.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(r#"{{"api_root": "{root}api/"}}"#))
            .expect(1)
            .create();
        let index = server.mock("GET", "/index.json").expect(0).create();
        let whoami = server.mock("GET", "/api/v1/whoami").expect(0).create();
        let mut store = InMemoryCredentialStore::new();

        let (outcome, notices) = run_login_with(&mut store, &server.url(), "tok", Some(false));

        let (_, _, validated) = stored_validated(outcome);
        assert!(validated.is_empty());
        assert!(notices.is_empty(), "unexpected notices: {notices:?}");
        config.assert();
        index.assert();
        whoami.assert();
        assert_eq!(store.list().unwrap()[0].secret, "tok");
    }
}
