// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::{
    MAX_ERROR_BODY_CHARS, PublishError, build_upload_url, is_valid_name, is_valid_publisher,
    normalize_field, summarize_error_body,
};
use url::Url;

#[test]
fn publisher_field_validation() {
    assert!(is_valid_publisher("Acme Labs"));
    assert!(is_valid_publisher("ACME-LABS-42"));
    assert!(is_valid_publisher("abc"));
    assert!(is_valid_publisher(
        "abcdefghijklmnopqrstuvxyzabcdefghijklmnopqrstuvxyz"
    ));
    assert!(!is_valid_publisher("ab"));
    assert!(!is_valid_publisher(
        "abcdefghijklmnopqrstuvxyzabcdefghijklmnopqrstuvxyza"
    ));
    assert!(!is_valid_publisher("Acme.Labs"));
    assert!(!is_valid_publisher("Åcme Labs"));
    assert!(!is_valid_publisher("Acme  Labs"));
    assert!(!is_valid_publisher("Acme. Labs"));
    assert!(!is_valid_publisher("Acme- Labs"));
    assert!(!is_valid_publisher("Acme__Labs"));
    assert!(!is_valid_publisher("Acme."));
}

#[test]
fn name_field_validation() {
    assert!(is_valid_name("My.Project Alpha"));
    assert!(is_valid_name("Alpha-2"));
    assert!(!is_valid_name("ab"));
    assert!(!is_valid_name("My..Project"));
    assert!(!is_valid_name("My__Project"));
    assert!(!is_valid_name(".Project"));
}

#[test]
fn normalize_field_preserves_dot() {
    assert_eq!(normalize_field("My.Project Alpha"), "my.project-alpha");
    assert_eq!(normalize_field("ACME LABS"), "acme-labs");
}

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
fn build_upload_url_strips_query_and_fragment() {
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
fn summarize_error_body_extracts_json_error_and_detail() {
    assert_eq!(
        summarize_error_body(br#"{"error":"Invalid token","detail":"Token not found or invalid"}"#),
        "Invalid token: Token not found or invalid"
    );
}

#[test]
fn summarize_error_body_falls_back_for_non_text_bytes() {
    assert_eq!(
        summarize_error_body(&[0, 159, 255]),
        "unexpected non-text error response (3 bytes)"
    );
}

#[test]
fn summarize_error_body_truncates_text_content() {
    let long = "x".repeat(MAX_ERROR_BODY_CHARS + 20);
    let summarized = summarize_error_body(long.as_bytes());
    assert!(summarized.ends_with(" ... [truncated]"));
    assert!(summarized.len() > MAX_ERROR_BODY_CHARS);
}
