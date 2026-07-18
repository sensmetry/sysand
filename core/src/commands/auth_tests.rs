// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use chrono::{TimeZone, Utc};

use super::{
    AuthCommandError, EnvCredentialEntry, StoredCredentialsStatus, assemble_auth_status,
    do_auth_logout, do_auth_status,
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
