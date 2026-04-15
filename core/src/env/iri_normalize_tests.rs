// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;

fn normalize(iri: &str) -> String {
    normalize_iri_for_hash(iri).expect("fixture IRI must normalize cleanly")
}

#[test]
fn scheme_and_host_lowercased() {
    assert_eq!(
        normalize("HTTP://EXAMPLE.com/path"),
        "http://example.com/path"
    );
}

#[test]
fn percent_encoded_unreserved_decoded_and_reserved_uppercased() {
    // %7E is `~` (unreserved — decoded); %2F is `/` (reserved — hex uppercased).
    assert_eq!(
        normalize("http://example.com/%7euser/a%2fb"),
        "http://example.com/~user/a%2Fb"
    );
}

#[test]
fn dot_segments_removed() {
    assert_eq!(
        normalize("http://example.com/a/./b/../c"),
        "http://example.com/a/c"
    );
}

#[test]
fn default_port_stripped() {
    // RFC 3986 §6.2.3 scheme-based normalization: `:80` on http is the
    // default and is removed. Same bucket as no port at all.
    assert_eq!(
        normalize("http://example.com:80/"),
        normalize("http://example.com/")
    );
}

#[test]
fn http_empty_path_becomes_slash() {
    // HTTP root-path fixup: `http://example.com` and `http://example.com/`
    // denote the same resource and must hash to the same bucket.
    assert_eq!(
        normalize("http://example.com"),
        normalize("http://example.com/")
    );
    assert_eq!(normalize("http://example.com"), "http://example.com/");
}

#[test]
fn http_empty_path_preserves_query_and_fragment() {
    assert_eq!(
        normalize("http://example.com?q=1"),
        "http://example.com/?q=1"
    );
    assert_eq!(
        normalize("http://example.com#top"),
        "http://example.com/#top"
    );
}

#[test]
fn ipv6_literal_canonicalized() {
    // RFC 5952 collapses `:0000:` runs to `::` and lowercases hex digits.
    assert_eq!(
        normalize("http://[2001:0DB8:0000:0000:0000:0000:0000:0001]/"),
        "http://[2001:db8::1]/"
    );
}

#[test]
fn idn_host_is_punycoded() {
    // `bücher.de` → `xn--bcher-kva.de`.
    assert_eq!(normalize("http://bücher.de/"), "http://xn--bcher-kva.de/");
}

#[test]
fn non_http_scheme_empty_path_preserved() {
    // The empty-path fixup is HTTP(S)-specific; other schemes leave the
    // empty path alone.
    assert_eq!(normalize("urn:kpar:b"), "urn:kpar:b");
}

#[test]
fn userinfo_and_non_default_port_preserved() {
    assert_eq!(
        normalize("http://User@Example.COM:8080/path"),
        "http://User@example.com:8080/path"
    );
}

#[test]
fn malformed_iri_is_rejected() {
    // Empty scheme, space in authority — not a valid IRI reference.
    assert!(normalize_iri_for_hash("http://exa mple.com/").is_err());
}
