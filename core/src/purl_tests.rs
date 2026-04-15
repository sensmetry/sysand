// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;

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
