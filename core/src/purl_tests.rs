// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;

#[test]
fn publisher_field_validation() {
    assert!(is_valid_unnormalized_publisher("Acme Labs"));
    assert!(is_valid_unnormalized_publisher("ACME-LABS-42"));
    assert!(is_valid_unnormalized_publisher("abc"));
    assert!(is_valid_unnormalized_publisher(
        "abcdefghijklmnopqrstuvxyzabcdefghijklmnopqrstuvxyz"
    ));
    // Digits are alphanumeric: leading, trailing, and all-digit segments
    // are valid (only `.` and the separator-position rules constrain
    // characters; `is_ascii_alphanumeric` covers `0-9`).
    assert!(is_valid_unnormalized_publisher("42-acme"));
    assert!(is_valid_unnormalized_publisher("acme-42"));
    assert!(is_valid_unnormalized_publisher("4cme"));
    assert!(is_valid_unnormalized_publisher("acm3"));
    assert!(is_valid_unnormalized_publisher("123"));
    assert!(is_valid_unnormalized_publisher("1a2"));
    assert!(!is_valid_unnormalized_publisher("ab"));
    assert!(!is_valid_unnormalized_publisher(
        "abcdefghijklmnopqrstuvxyzabcdefghijklmnopqrstuvxyza"
    ));
    assert!(!is_valid_unnormalized_publisher("Acme.Labs"));
    assert!(!is_valid_unnormalized_publisher("Åcme Labs"));
    assert!(!is_valid_unnormalized_publisher("Acme  Labs"));
    assert!(!is_valid_unnormalized_publisher("Acme. Labs"));
    assert!(!is_valid_unnormalized_publisher("Acme- Labs"));
    assert!(!is_valid_unnormalized_publisher("Acme__Labs"));
    assert!(!is_valid_unnormalized_publisher("Acme."));
}

#[test]
fn name_field_validation() {
    assert!(is_valid_unnormalized_name("My.Project Alpha"));
    assert!(is_valid_unnormalized_name("Alpha-2"));
    // Digits are alphanumeric: leading, trailing, and all-digit names are
    // accepted, including digits adjacent to the dot separator that names
    // additionally allow.
    assert!(is_valid_unnormalized_name("3d-printer"));
    assert!(is_valid_unnormalized_name("v1.0"));
    assert!(is_valid_unnormalized_name("2project"));
    assert!(is_valid_unnormalized_name("project2"));
    assert!(is_valid_unnormalized_name("123"));
    assert!(is_valid_unnormalized_name("1.2"));
    assert!(!is_valid_unnormalized_name("ab"));
    assert!(!is_valid_unnormalized_name("My..Project"));
    assert!(!is_valid_unnormalized_name("My__Project"));
    assert!(!is_valid_unnormalized_name(".Project"));
}

#[test]
fn normalize_field_preserves_dot() {
    assert_eq!(normalize_field("My.Project Alpha"), "my.project-alpha");
    assert_eq!(normalize_field("ACME LABS"), "acme-labs");
}

#[test]
fn parse_sysand_purl_recognises_other_schemes_as_not_sysand_purl() {
    assert_eq!(parse_sysand_purl("urn:kpar:foo"), Ok(None));
    assert_eq!(parse_sysand_purl("https://example.com/x"), Ok(None));
    assert_eq!(parse_sysand_purl("pkg:npm/lodash"), Ok(None));
}

#[test]
fn parse_sysand_purl_accepts_normalized_two_segment() {
    assert_eq!(
        parse_sysand_purl("pkg:sysand/admin/proj0"),
        Ok(Some(("admin", "proj0")))
    );
    assert_eq!(
        parse_sysand_purl("pkg:sysand/acme-labs/my.project"),
        Ok(Some(("acme-labs", "my.project")))
    );
}

#[test]
fn parse_sysand_purl_rejects_wrong_segment_count() {
    assert_eq!(
        parse_sysand_purl("pkg:sysand/"),
        Err(SysandPurlError::WrongShape {
            purl: "pkg:sysand/".to_owned(),
            segments: 1
        })
    );
    assert_eq!(
        parse_sysand_purl("pkg:sysand/a"),
        Err(SysandPurlError::WrongShape {
            purl: "pkg:sysand/a".to_owned(),
            segments: 1
        })
    );
    assert_eq!(
        parse_sysand_purl("pkg:sysand/a/b/c"),
        Err(SysandPurlError::WrongShape {
            purl: "pkg:sysand/a/b/c".to_owned(),
            segments: 3
        })
    );
    // Trailing-slash form parses to two segments (`["a", ""]`); the publisher
    // is too short, so we reject on InvalidPublisher rather than WrongShape.
    assert!(matches!(
        parse_sysand_purl("pkg:sysand/a/"),
        Err(SysandPurlError::InvalidPublisher { .. })
    ));
}

#[test]
fn parse_sysand_purl_rejects_traversal_and_dot_publishers() {
    assert!(matches!(
        parse_sysand_purl("pkg:sysand/../attacker"),
        Err(SysandPurlError::WrongShape { .. } | SysandPurlError::InvalidPublisher { .. })
    ));
    assert!(matches!(
        parse_sysand_purl("pkg:sysand/.hidden/proj"),
        Err(SysandPurlError::InvalidPublisher { .. })
    ));
    assert!(matches!(
        parse_sysand_purl("pkg:sysand/pub/.hidden"),
        Err(SysandPurlError::InvalidName { .. })
    ));
}

#[test]
fn parse_sysand_purl_rejects_non_normalized_with_suggestion() {
    let err = parse_sysand_purl("PKG:sysand/Admin/Proj0").unwrap_err();
    let SysandPurlError::NotNormalized {
        purl,
        norm_publisher: publisher,
        norm_name: name,
    } = err
    else {
        panic!("expected NotNormalized, got {err:?}");
    };
    assert_eq!(purl, "PKG:sysand/Admin/Proj0");
    assert_eq!(publisher, "admin");
    assert_eq!(name, "proj0");

    // `sysand` segment is case-sensitive
    let res = parse_sysand_purl("PKG:SysAnd/admin/proj0").unwrap();
    assert_eq!(res, None);

    let err = parse_sysand_purl("pkg:sysand/Acme Labs/My.Project").unwrap_err();
    let SysandPurlError::NotNormalized {
        purl,
        norm_publisher: publisher,
        norm_name: name,
    } = err
    else {
        panic!("expected NotNormalized, got {err:?}");
    };
    assert_eq!(purl, "pkg:sysand/Acme Labs/My.Project");
    assert_eq!(publisher, "acme-labs");
    assert_eq!(name, "my.project");
}

#[test]
fn parse_sysand_purl_error_messages_include_input_purl() {
    let err = parse_sysand_purl("pkg:sysand/ab/proj0").unwrap_err();
    assert!(err.to_string().contains("`pkg:sysand/ab/proj0`"), "{err}");

    let err = parse_sysand_purl("pkg:sysand/Acme Labs/My.Project").unwrap_err();
    assert!(
        err.to_string()
            .contains("`pkg:sysand/Acme Labs/My.Project`"),
        "{err}"
    );
}

#[test]
fn parse_sysand_purl_rejects_non_ascii_and_invalid_chars() {
    assert!(matches!(
        parse_sysand_purl("pkg:sysand/Åcme/proj"),
        Err(SysandPurlError::InvalidPublisher { .. })
    ));
    assert!(matches!(
        parse_sysand_purl("pkg:sysand/pub\t/proj"),
        Err(SysandPurlError::InvalidPublisher { .. })
    ));
    assert!(matches!(
        parse_sysand_purl("pkg:sysand/ab/proj0"),
        Err(SysandPurlError::InvalidPublisher { .. })
    ));
    assert!(matches!(
        parse_sysand_purl("pkg:sysand/aąčb/oroūj0"),
        Err(SysandPurlError::InvalidPublisher { .. })
    ));
}
