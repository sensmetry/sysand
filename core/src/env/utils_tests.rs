use std::{cmp::min, error::Error};

use fluent_uri::Iri;

use super::*;

// TODO: test normalization, canonicalization and candidate generation

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
fn disambiguation() -> Result<(), Box<dyn Error>> {
    let iri = Iri::parse("scheme:abc/def")?;
    let version = "1.0.0";

    let mut name = IriVersionFilename::new(&iri, version);

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

        let mut name = IriVersionFilename::new(&iri, version);

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
        // Punycode and case folding works
        ("http://ąčĘė", "ąčęė"),
    ] {
        let iri = Iri::parse(iri)?;
        let normalized = normalize_iri(&iri);
        assert_eq!(normalized, expected);
    }
    Ok(())
}
