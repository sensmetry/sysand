// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use chrono::{TimeZone as _, Utc};

use std::assert_matches;

use super::{
    BLOB_VERSION, CredentialBlob, CredentialRecord, CredentialScheme, CredentialStoreError,
    CredentialSubject, SubjectKind, ValidatedSurface, parse_blob, serialize_blob,
};

fn record(key: &str, secret: &str) -> CredentialRecord {
    CredentialRecord {
        key: key.to_owned(),
        globs: vec![format!("{key}**")],
        scheme: CredentialScheme::Bearer,
        secret: secret.to_owned(),
        expires_at: None,
        subject: None,
        token_name: None,
        token_prefix: None,
        validated: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

#[test]
fn blob_round_trip_and_version_field() {
    let mut with_expiry = record("https://example.com/idx/", "tok-1");
    with_expiry.expires_at = Some(Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap());
    let blob = CredentialBlob::new(vec![with_expiry, record("https://other.example/", "tok-2")]);
    let raw = serialize_blob(&blob);
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["version"], serde_json::json!(BLOB_VERSION));
    let parsed = parse_blob(&raw).unwrap();
    assert_eq!(parsed, blob);
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
    let rewritten = serialize_blob(&blob);
    assert!(rewritten.contains("future_top_level"));
    assert!(rewritten.contains("future_record_field"));
}

#[test]
fn parse_accepts_a_blob_written_before_the_newer_fields() {
    // A blob written before `subject` / `token_name` / `token_prefix` /
    // `validated` existed (still version 1) must parse, with the newer
    // fields absent and the absent claim reading as "not validated".
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
    assert!(record.validated.is_empty());
    assert!(record.expires_at.is_some());
    assert!(record.extra.is_empty());
}

#[test]
fn identity_fields_round_trip() {
    let mut with_identity = record("https://example.com/idx/", "tok");
    with_identity.subject = Some(CredentialSubject {
        kind: SubjectKind::User,
        name: "alice".to_owned(),
    });
    with_identity.token_name = Some("laptop".to_owned());
    with_identity.token_prefix = Some("sysand_u_1a2b3c4d".to_owned());
    with_identity.validated = vec![ValidatedSurface::Read, ValidatedSurface::Api];
    let blob = CredentialBlob::new(vec![with_identity]);
    let raw = serialize_blob(&blob);
    // `kind` serializes under the protocol's `type` key.
    assert!(raw.contains(r#""subject":{"type":"user","name":"alice"}"#));
    // The validation claim serializes compactly.
    assert!(raw.contains(r#""validated":["read","api"]"#));
    assert_eq!(parse_blob(&raw).unwrap(), blob);
}

#[test]
fn parse_fails_closed_on_unknown_version_with_a_dedicated_error() {
    // An unknown version is not corruption: the dedicated error names
    // both versions and never suggests resetting the store.
    let raw = r#"{"version": 2, "credentials": []}"#;
    let err = parse_blob(raw).unwrap_err();
    assert_matches!(
        err,
        CredentialStoreError::UnsupportedBlobVersion {
            found: 2,
            expected: BLOB_VERSION,
        }
    );
    let message = err.to_string();
    assert!(
        message.contains("version 2") && message.contains("supports version 1"),
        "message must name both versions: {message}"
    );
    assert!(
        !message.contains("remove"),
        "must not suggest a reset: {message}"
    );
}

#[test]
fn unknown_subject_kind_and_surface_round_trip_unchanged() {
    // Values a newer server or sysand wrote must parse as `Other` and
    // survive an older binary's read-modify-write byte-for-byte.
    let raw = r#"{
        "version": 1,
        "credentials": [{
            "key": "https://example.com/",
            "globs": ["https://example.com/**"],
            "scheme": "bearer",
            "secret": "tok",
            "subject": {"type": "robot", "name": "bot-7"},
            "validated": ["read", "novel-surface"]
        }]
    }"#;
    let blob = parse_blob(raw).unwrap();
    let record = &blob.credentials[0];
    assert_eq!(
        record.subject,
        Some(CredentialSubject {
            kind: SubjectKind::Other("robot".to_owned()),
            name: "bot-7".to_owned(),
        })
    );
    assert_eq!(
        record.validated,
        vec![
            ValidatedSurface::Read,
            ValidatedSurface::Other("novel-surface".to_owned()),
        ]
    );
    // The canonical string form renders unknown values verbatim.
    assert_eq!(record.subject.as_ref().unwrap().kind.to_string(), "robot");
    assert_eq!(record.validated[1].to_string(), "novel-surface");
    let rewritten = serialize_blob(&blob);
    assert!(rewritten.contains(r#""subject":{"type":"robot","name":"bot-7"}"#));
    assert!(rewritten.contains(r#""validated":["read","novel-surface"]"#));
    assert_eq!(parse_blob(&rewritten).unwrap(), blob);
}

#[test]
fn parse_fails_closed_on_garbage() {
    for garbage in ["", "not json", "{\"version\": \"one\"}", "[]", "{}"] {
        let err = parse_blob(garbage).unwrap_err();
        assert_matches!(
            err,
            CredentialStoreError::Unreadable,
            "input {garbage:?} must fail closed"
        );
        assert!(
            err.to_string()
                .contains("remove the `sysand` keyring entry"),
            "message must point at the reset: {err}"
        );
    }
}
