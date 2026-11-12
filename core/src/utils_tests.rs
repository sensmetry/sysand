// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::utils::is_newer_format_version;

#[test]
fn a_higher_minor_or_major_is_newer() {
    assert!(is_newer_format_version("0.6", "0.5"));
    assert!(is_newer_format_version("0.10", "0.9"));
    assert!(is_newer_format_version("1.0", "0.9"));
}

#[test]
fn the_current_and_older_versions_are_not_newer() {
    assert!(!is_newer_format_version("0.5", "0.5"));
    assert!(!is_newer_format_version("0.3", "0.5"));
    assert!(!is_newer_format_version("0.9", "0.10"));
}

#[test]
fn anything_but_two_integers_is_unknown_rather_than_newer() {
    for version in ["X", "1", "0.5.1", "a.b", "", "9.", ".9"] {
        assert!(!is_newer_format_version(version, "0.5"), "{version:?}");
    }
    assert!(!is_newer_format_version("0.6", "x.y"));
}
