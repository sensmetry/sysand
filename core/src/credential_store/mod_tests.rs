// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use chrono::{TimeZone, Utc};

use super::{
    BLOB_VERSION, CredentialBlob, CredentialRecord, CredentialScheme, CredentialStore,
    CredentialStoreError, CredentialSubject, InMemoryCredentialStore, normalize_index_key,
    parse_blob, serialize_blob,
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

#[test]
fn blob_round_trip() {
    let mut with_expiry = record("https://example.com/idx/", "tok-1");
    with_expiry.expires_at = Some(Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap());
    let blob = CredentialBlob::new(vec![with_expiry, record("https://other.example/", "tok-2")]);
    let raw = serialize_blob(&blob).unwrap();
    let parsed = parse_blob(&raw).unwrap();
    assert_eq!(parsed, blob);
}

#[test]
fn blob_serializes_version_field() {
    let raw = serialize_blob(&CredentialBlob::empty()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["version"], serde_json::json!(BLOB_VERSION));
}

#[test]
fn blob_omits_absent_expiry() {
    let blob = CredentialBlob::new(vec![record("https://example.com/", "tok")]);
    let raw = serialize_blob(&blob).unwrap();
    assert!(!raw.contains("expires_at"));
}

#[test]
fn parse_tolerates_unknown_fields_and_round_trips_them() {
    let raw = r#"{
        "version": 1,
        "future_top_level": true,
        "credentials": [{
            "key": "https://example.com/",
            "globs": ["https://example.com/**"],
            "scheme": "bearer",
            "secret": "tok",
            "future_record_field": {"nested": 1}
        }]
    }"#;
    let blob = parse_blob(raw).unwrap();
    assert_eq!(blob.credentials.len(), 1);
    assert_eq!(
        blob.credentials[0].extra["future_record_field"],
        serde_json::json!({"nested": 1})
    );
    assert_eq!(blob.extra["future_top_level"], serde_json::json!(true));
    // A read-modify-write must not drop fields a newer sysand wrote.
    let rewritten = serialize_blob(&blob).unwrap();
    assert!(rewritten.contains("future_top_level"));
    assert!(rewritten.contains("future_record_field"));
}

#[test]
fn parse_accepts_a_blob_written_before_the_identity_fields() {
    // A blob written before `subject` / `token_name` / `token_prefix`
    // existed (still version 1) must parse, with the new fields absent.
    let raw = r#"{
        "version": 1,
        "credentials": [{
            "key": "https://example.com/",
            "globs": ["https://example.com/**"],
            "scheme": "bearer",
            "secret": "tok",
            "expires_at": "2026-09-01T00:00:00Z"
        }]
    }"#;
    let blob = parse_blob(raw).unwrap();
    let record = &blob.credentials[0];
    assert_eq!(record.secret, "tok");
    assert_eq!(record.subject, None);
    assert_eq!(record.token_name, None);
    assert_eq!(record.token_prefix, None);
    assert!(record.expires_at.is_some());
    assert!(record.extra.is_empty());
}

#[test]
fn identity_fields_round_trip() {
    let mut with_identity = record("https://example.com/idx/", "tok");
    with_identity.subject = Some(CredentialSubject {
        kind: "user".to_string(),
        name: "alice".to_string(),
    });
    with_identity.token_name = Some("laptop".to_string());
    with_identity.token_prefix = Some("sysand_u_1a2b3c4d".to_string());
    let blob = CredentialBlob::new(vec![with_identity]);
    let raw = serialize_blob(&blob).unwrap();
    // `kind` serializes under the protocol's `type` key.
    assert!(raw.contains(r#""subject":{"type":"user","name":"alice"}"#));
    assert_eq!(parse_blob(&raw).unwrap(), blob);
}

#[test]
fn parse_fails_closed_on_unknown_version() {
    let raw = r#"{"version": 2, "credentials": []}"#;
    let err = parse_blob(raw).unwrap_err();
    assert!(matches!(err, CredentialStoreError::Unreadable));
}

#[test]
fn parse_fails_closed_on_garbage() {
    for garbage in ["", "not json", "{\"version\": \"one\"}", "[]", "{}"] {
        let err = parse_blob(garbage).unwrap_err();
        assert!(
            matches!(err, CredentialStoreError::Unreadable),
            "input {garbage:?} must fail closed"
        );
        assert_eq!(
            err.to_string(),
            "credential store unreadable; remove the `sysand` keyring entry to reset"
        );
    }
}

#[test]
fn normalize_adds_trailing_slash() {
    assert_eq!(
        normalize_index_key("https://example.com/idx").unwrap(),
        "https://example.com/idx/"
    );
    assert_eq!(
        normalize_index_key("https://example.com").unwrap(),
        "https://example.com/"
    );
}

#[test]
fn normalize_keeps_existing_trailing_slash() {
    assert_eq!(
        normalize_index_key("https://example.com/idx/").unwrap(),
        "https://example.com/idx/"
    );
}

#[test]
fn normalize_lowercases_host_and_strips_default_port() {
    assert_eq!(
        normalize_index_key("HTTPS://EXAMPLE.com:443/Idx").unwrap(),
        "https://example.com/Idx/"
    );
}

#[test]
fn normalize_keeps_explicit_port_and_ipv6_literal() {
    assert_eq!(
        normalize_index_key("https://[::1]:8000/idx").unwrap(),
        "https://[::1]:8000/idx/"
    );
}

#[test]
fn normalize_drops_fragment() {
    assert_eq!(
        normalize_index_key("https://example.com/idx#frag").unwrap(),
        "https://example.com/idx/"
    );
}

#[test]
fn normalize_rejects_non_http_schemes() {
    for url in ["file:///tmp/idx", "ftp://example.com/idx", "not a url"] {
        assert!(
            matches!(
                normalize_index_key(url),
                Err(CredentialStoreError::InvalidIndexUrl(_))
            ),
            "url {url:?} must be rejected"
        );
    }
}

#[test]
fn normalize_rejects_query_and_userinfo() {
    for url in [
        "https://example.com/idx?token=x",
        "https://user:pass@example.com/idx",
        "https://user@example.com/idx",
    ] {
        assert!(
            matches!(
                normalize_index_key(url),
                Err(CredentialStoreError::InvalidIndexUrl(_))
            ),
            "url {url:?} must be rejected"
        );
    }
}

#[test]
fn in_memory_store_upsert_list_remove() {
    let mut store = InMemoryCredentialStore::new();
    assert!(store.list().unwrap().is_empty());

    store.upsert(record("https://a.example/", "tok-a")).unwrap();
    store.upsert(record("https://b.example/", "tok-b")).unwrap();
    assert_eq!(store.list().unwrap().len(), 2);

    // Upsert replaces by key without duplicating.
    store
        .upsert(record("https://a.example/", "tok-a2"))
        .unwrap();
    let records = store.list().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].secret, "tok-a2");

    assert!(store.remove("https://a.example/").unwrap());
    assert!(!store.remove("https://a.example/").unwrap());
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(store.list().unwrap()[0].key, "https://b.example/");
}
