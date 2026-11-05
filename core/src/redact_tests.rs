// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use url::Url;

use super::{RedactedUrl, ResponseUrl};

fn redacted(url: &str) -> String {
    RedactedUrl::from(Url::parse(url).unwrap()).to_string()
}

#[test]
fn password_is_masked_and_username_kept() {
    assert_eq!(
        redacted("https://user:secret@example.com/a"),
        "https://user:****@example.com/a"
    );
}

/// A username without a password is often a token.
#[test]
fn lone_username_is_masked() {
    assert_eq!(
        redacted("https://token@example.com/a"),
        "https://****@example.com/a"
    );
}

#[test]
fn sensitive_query_values_are_masked_and_others_kept() {
    assert_eq!(
        redacted(
            "https://example.com/a.kpar?X-Amz-Signature=abc&x-amz-credential=def&ref=main#frag"
        ),
        "https://example.com/a.kpar?X-Amz-Signature=****&x-amz-credential=****&ref=main#frag"
    );
    assert_eq!(
        redacted("https://example.com/a.kpar?sv=2024&sig=abc"),
        "https://example.com/a.kpar?sv=2024&sig=****"
    );
    assert_eq!(
        redacted("https://example.com/a.kpar?X-Goog-Credential=abc&X-Goog-Signature=def"),
        "https://example.com/a.kpar?X-Goog-Credential=****&X-Goog-Signature=****"
    );
    assert_eq!(
        redacted("https://example.com/a.kpar?Expires=1&Signature=abc&Key-Pair-Id=def&Policy=ghi"),
        "https://example.com/a.kpar?Expires=1&Signature=****&Key-Pair-Id=****&Policy=****"
    );
}

/// Only masked values change; other parameters keep their encoding.
#[test]
fn other_query_parameters_keep_their_encoding() {
    assert_eq!(
        redacted("https://example.com/a?q=a%20b&r=c+d&flag&sig=abc"),
        "https://example.com/a?q=a%20b&r=c+d&flag&sig=****"
    );
}

#[test]
fn url_without_credentials_is_unchanged() {
    let url = "https://example.com/a/b.kpar?ref=main%2Fx#frag";
    assert_eq!(redacted(url), url);
}

#[test]
fn debug_is_redacted() {
    let url = RedactedUrl::from(Url::parse("https://user:secret@example.com/?sig=abc").unwrap());
    let debug = format!("{url:?}");
    assert!(
        !debug.contains("secret") && !debug.contains("abc"),
        "{debug}"
    );
}

#[test]
fn response_url_names_a_redirect_target_only_if_different() {
    let requested = Url::parse("https://user:secret@example.com/a").unwrap();
    let elsewhere = Url::parse("https://cdn.example.com/a?sig=abc").unwrap();

    assert_eq!(
        ResponseUrl::new(&requested, &requested).to_string(),
        "`https://user:****@example.com/a`"
    );
    assert_eq!(
        ResponseUrl::new(&requested, &elsewhere).to_string(),
        "`https://user:****@example.com/a` (redirected to `https://cdn.example.com/a?sig=****`)"
    );
}

#[cfg(feature = "networking")]
#[test]
fn http_error_renders_without_its_url() {
    use std::net::TcpListener;

    use super::HttpError;
    use crate::resolve::net_utils::{client_builder, install_default_crypto_provider};

    install_default_crypto_provider();
    // Nothing listens on a port that was bound and released
    let addr = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let url = format!("http://user:secret@{addr}/a.kpar?X-Amz-Signature=abc");
    let err = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(client_builder().build().unwrap().get(&url).send())
        .unwrap_err();

    let rendered = crate::utils::format_err(HttpError::from(err));
    assert!(!rendered.contains(&addr.to_string()), "{rendered}");
    assert!(
        rendered.starts_with("error sending request\n"),
        "{rendered}"
    );
}
