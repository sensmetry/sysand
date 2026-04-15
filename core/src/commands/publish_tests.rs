// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::{PublishError, build_upload_url, error_body_to_string};
use url::Url;

#[test]
fn build_upload_url_appends_endpoint_path() {
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/api/v1/upload"
    );
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/api/v1/upload"
    );
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/index").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/index/api/v1/upload"
    );
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/index/").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/index/api/v1/upload"
    );
}

#[test]
fn build_upload_url_preserves_percent_encoded_segments() {
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/my%20index/").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/my%20index/api/v1/upload"
    );
}

#[test]
fn build_upload_url_rejects_upload_endpoint_path() {
    for url in [
        "https://example.org/api/v1/upload",
        "https://example.org/api/v1/upload/",
        "https://example.org/index/api/v1/upload",
    ] {
        let err = build_upload_url(&Url::parse(url).unwrap()).unwrap_err();
        assert!(matches!(err, PublishError::InvalidIndexUrl { .. }));
    }
}

#[test]
fn build_upload_url_rejects_query_and_fragment() {
    let err =
        build_upload_url(&Url::parse("https://example.org/index?x=1#frag").unwrap()).unwrap_err();
    assert!(matches!(err, PublishError::InvalidIndexUrl { .. }));
}

#[test]
fn build_upload_url_rejects_non_http_scheme() {
    let err = build_upload_url(&Url::parse("ftp://example.org").unwrap()).unwrap_err();
    assert!(matches!(err, PublishError::InvalidIndexUrl { .. }));
}

#[test]
fn build_upload_url_rejects_non_hierarchical_url() {
    let err = build_upload_url(&Url::parse("mailto:test@example.org").unwrap()).unwrap_err();
    assert!(matches!(err, PublishError::InvalidIndexUrl { .. }));
}

#[test]
fn error_body_to_string_trims_text_content() {
    assert_eq!(error_body_to_string(b"  unauthorized\n"), "unauthorized");
}

#[test]
fn error_body_to_string_extracts_error_from_json() {
    assert_eq!(
        error_body_to_string(br#"{"error":"Invalid token"}"#),
        "Invalid token"
    );
}

#[test]
fn error_body_to_string_reports_empty_body() {
    assert_eq!(error_body_to_string(b" \n\t "), "no error details provided");
}
