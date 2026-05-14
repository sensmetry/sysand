// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;
use fluent_uri::Iri;

fn normalize(iri: &str) -> String {
    let parsed = Iri::parse(iri).expect("fixture IRI must parse cleanly");
    canonicalize_iri(parsed).expect("fixture IRI must normalize cleanly")
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
use std::{cmp::min, error::Error};

/// Returns (IRI, expected_normalized)
fn gen_iri(len: u32) -> (Iri<String>, String) {
    let mut iri = String::from("urn:kpar:");
    let mut normalized = String::from("kpar.");
    for i in 0..len {
        iri.push('a');
        if (i as usize) < 120 - "kpar.".len() {
            normalized.push('a');
        }
    }
    (Iri::parse(iri).unwrap(), normalized)
}

#[test]
fn create_filename() -> Result<(), Box<dyn Error>> {
    let iri = Iri::parse("urn:kpar:sysmod")?;
    let version = "5.0.0-alpha.2";

    let mut name = IriVersionFilename::new(iri, version);

    assert_eq!(name.next_candidate(), "kpar.sysmod_5.0.0-alpha.2");

    Ok(())
}

#[test]
fn disambiguation() -> Result<(), Box<dyn Error>> {
    let iri = Iri::parse("scheme:abc/def")?;
    let version = "1.0.0";

    let mut name = IriVersionFilename::new(iri, version);

    assert_eq!(name.next_candidate(), "abc.def_1.0.0");
    for i in 1..105 {
        let expected = format!("abc.def_{i}_1.0.0");
        assert_eq!(name.next_candidate(), expected);
    }

    Ok(())
}

#[test]
fn iri_truncation() -> Result<(), Box<dyn Error>> {
    for len in [1, 10, 100, 116, 117, 118, 119, 120, 121, 122, 123] {
        let (iri, mut expected_normalized) = gen_iri(len);
        let version = "1.0.0";
        expected_normalized.push_str("_1.0.0");

        let mut name = IriVersionFilename::new(iri.borrow(), version);

        assert_eq!(name.next_candidate(), expected_normalized);
    }

    Ok(())
}

#[test]
fn version_truncation() -> Result<(), Box<dyn Error>> {
    let base_len = "1.0.0-".len();
    for version_len in [1, 10, 29 - base_len, 30 - base_len, 31 - base_len, 40] {
        let version = format!("1.0.0-{}", "a".repeat(version_len));
        let expected_len = min(version.len(), 30);
        let normalized = normalize_version(version);
        assert_eq!(normalized.len(), expected_len);
    }

    Ok(())
}

#[test]
fn misc_iris() -> Result<(), Box<dyn Error>> {
    for (iri, expected) in [
        // Strips scheme
        ("http://example.com", "example.com"),
        ("https://aaa.example.org/project", "aaa.example.org-project"),
        (
            "https://aaa.example.org/project.kpar",
            "aaa.example.org-project",
        ),
        // Properly preserves non-ASCII
        (
            "ssh://example.org/Mekanïk/Kommandöh",
            "example.org-mekanïk.kommandöh",
        ),
        // Strips end punctuation
        ("a:b.", "b"),
        // Removes invalid bytes
        ("urn:kpar:%FF", "kpar"),
        // Does not remove valid bytes before/after invalid
        ("urn:kpar:aaž%FFąčę123%20", "kpar.aažąčę123"),
        // Lowercasing works with percent-encoding
        ("a:b%42", "bb"),
        // Decode array correctly deals with multiple multibyte char start bytes
        ("a:b%F0%F0%F0%F0%F0c", "bc"),
        // Multibyte chars split among two disjoint chunks are stripped
        ("ab:b%F0%9F%92C%96", "bc"),
        // Punycode and case folding works
        ("http://ąčĘė", "ąčęė"),
        // Replacement character is stripped
        ("a:b%EF%BF%BDcd", "b-cd"),
    ] {
        let iri = Iri::parse(iri)?;
        let normalized = iri_to_filename_part(iri);
        assert_eq!(normalized, expected);
    }
    Ok(())
}

#[test]
fn misc_versions() -> Result<(), Box<dyn Error>> {
    for (version, expected) in [
        ("1.2.3", "1.2.3"),
        ("1.2.3-alpha1", "1.2.3-alpha1"),
        ("1.2.3-a.b.c", "1.2.3-a.b.c"),
        // Allows arbitrary contents, lowercases
        ("Mekanïk/Kommandöh", "mekanïk-kommandöh"),
        // Strips end punctuation
        ("1.", "1"),
        ("0.0.1-a+build1", "0.0.1-a-build1"),
        (".1.2", "1.2"),
        // Replacement character is stripped
        ("abc�d.1", "abc-d.1"),
    ] {
        let normalized = normalize_version(version);
        assert_eq!(normalized, expected);
    }
    Ok(())
}

// This should not pass, because name `con.<ext>` is invalid on Windows.
// TODO: if semver is mandated, this issue is impossible to hit, because
// version always contains two dots. Can we mandate semver here?
#[test]
fn invalid_windows_name() -> Result<(), Box<dyn Error>> {
    let iri = Iri::parse("a:con.b")?;
    let version = "a";
    let mut name_gen = IriVersionFilename::new(iri, version);
    assert_eq!(name_gen.next_candidate(), "con.b_a");

    Ok(())
}
