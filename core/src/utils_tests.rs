// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use typed_path::Utf8UnixPathBuf;

use crate::utils::{NormalizeRelativeError, is_newer_format_version, normalize_relative};

fn normalized(path: &str) -> Utf8UnixPathBuf {
    normalize_relative(path).expect("path should normalize")
}

fn error(path: &str) -> NormalizeRelativeError {
    normalize_relative(path).expect_err("path should be rejected")
}

#[test]
fn normalize_relative_resolves_dot_segments() {
    assert_eq!(normalized("./a"), Utf8UnixPathBuf::from("a"));
    assert_eq!(normalized("a/./b"), Utf8UnixPathBuf::from("a/b"));
    assert_eq!(normalized("a/../b"), Utf8UnixPathBuf::from("b"));
    assert_eq!(normalized("a//b"), Utf8UnixPathBuf::from("a/b"));
    assert_eq!(
        normalized("./src/../lib//util.kerml"),
        Utf8UnixPathBuf::from("lib/util.kerml")
    );
    // A trailing slash is dropped here; the callers that care about a
    // directory path (`Sources`) reject it before normalizing.
    assert_eq!(normalized("a/"), Utf8UnixPathBuf::from("a"));
}

#[test]
fn normalize_relative_rejects_absolute_paths() {
    for path in ["/x", "//x", "C:/x", "C:\\x"] {
        assert!(
            matches!(error(path), NormalizeRelativeError::Absolute { path: _ }),
            "{path} should be absolute"
        );
    }
}

#[test]
fn normalize_relative_rejects_windows_separators_and_nul() {
    for path in ["a\\b", "a\\\\b", "src\\main.sysml"] {
        assert!(
            matches!(
                error(path),
                NormalizeRelativeError::ContainsBackslash { path: _ }
            ),
            "{path} should be rejected for its backslash"
        );
    }
    assert!(matches!(
        error("a\0b"),
        NormalizeRelativeError::ContainsNull { path: _ }
    ));
}

#[test]
fn normalize_relative_rejects_escaping_paths() {
    for path in ["../x", "a/../../x", "./../x"] {
        assert!(
            matches!(error(path), NormalizeRelativeError::EscapesRoot { path: _ }),
            "{path} should escape the root"
        );
    }
}

#[test]
fn normalize_relative_rejects_the_root_itself() {
    for path in [".", "", "a/..", "a/b/../..", "./"] {
        assert!(
            matches!(error(path), NormalizeRelativeError::IsRoot { path: _ }),
            "{path} should resolve to the root"
        );
    }
}

#[test]
fn normalize_relative_errors_quote_the_path() {
    assert_eq!(
        error("/x").to_string(),
        "path `/x` is absolute, but a path relative to the root was expected"
    );
    assert_eq!(
        error("../x").to_string(),
        "path `../x` points above the root it is relative to"
    );
    assert_eq!(
        error(".").to_string(),
        "path `.` is the root it is relative to, not a path inside it"
    );
}

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
