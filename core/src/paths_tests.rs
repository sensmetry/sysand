// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;
use std::error::Error;

use camino::Utf8Path;
use typed_path::Utf8UnixPathBuf;

use super::*;

fn normalized(path: &str) -> Utf8UnixPathBuf {
    normalize_relative(path).expect("path should normalize")
}

fn error(path: &str) -> NormalizeError {
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
    // A trailing slash is dropped here, so a directory path that must not
    // have one has to be rejected before normalizing.
    assert_eq!(normalized("a/"), Utf8UnixPathBuf::from("a"));
}

#[test]
fn normalize_relative_rejects_absolute_paths() {
    for path in ["/x", "//x", "C:/x", "C:\\x"] {
        assert_matches!(
            error(path),
            NormalizeError::Absolute { path: _ },
            "{path} should be absolute"
        );
    }
}

#[test]
fn normalize_relative_rejects_windows_separators_and_nul() {
    for path in ["a\\b", "a\\\\b", "src\\main.sysml"] {
        assert_matches!(
            error(path),
            NormalizeError::ContainsBackslash { path: _ },
            "{path} should be rejected for its backslash"
        );
    }
    assert_matches!(error("a\0b"), NormalizeError::ContainsNull { path: _ });
}

#[test]
fn normalize_relative_rejects_escaping_paths() {
    for path in ["../x", "a/../../x", "./../x"] {
        assert_matches!(
            error(path),
            NormalizeError::EscapesRoot { path: _ },
            "{path} should escape the root"
        );
    }
}

#[test]
fn normalize_relative_rejects_the_root_itself() {
    for path in [".", "", "a/..", "a/b/../..", "./"] {
        assert_matches!(
            error(path),
            NormalizeError::IsRoot { path: _ },
            "{path} should resolve to the root"
        );
    }
}

#[test]
fn normalize_relative_errors_quote_the_path() {
    assert_eq!(error("/x").to_string(), "path `/x` is absolute");
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
fn an_absolute_path_is_admitted_only_where_asked_for() {
    for path in ["/opt/lib", "C:/libs/lib"] {
        assert_matches!(
            parse_relative_unix_path(path, RelativePathKind::File),
            Err(RelativeUnixPathError::Absolute { path: _ }),
            "`{path}`"
        );
        assert_eq!(
            parse_unix_path_or_absolute(path, RelativePathKind::File)
                .unwrap()
                .as_str(),
            path
        );
    }
}

#[test]
fn an_absolute_path_is_held_to_the_rules_past_its_root() {
    assert_matches!(
        parse_unix_path_or_absolute("/opt//lib", RelativePathKind::File),
        Err(RelativeUnixPathError::ContainsDoubleSlash { path: _ })
    );
    assert_matches!(
        parse_unix_path_or_absolute("//opt/lib", RelativePathKind::File),
        Err(RelativeUnixPathError::ContainsDoubleSlash { path: _ })
    );
    assert_matches!(
        parse_unix_path_or_absolute("/opt/lib/", RelativePathKind::File),
        Err(RelativeUnixPathError::DirectoryPath { path: _ })
    );
    assert_matches!(
        parse_unix_path_or_absolute("/opt/..", RelativePathKind::SubDirectory),
        Err(RelativeUnixPathError::ContainsParent { path: _ })
    );
}

#[test]
fn simple_relativize_path() -> Result<(), Box<dyn Error>> {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\")
    } else {
        Utf8Path::new("/")
    };
    assert_eq!(relativize_path(path, root)?, "a/b/c");
    Ok(())
}

#[test]
fn backtracking_relativize_path() -> Result<(), Box<dyn Error>> {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\d\e\f")
    } else {
        Utf8Path::new("/d/e/f")
    };
    assert_eq!(relativize_path(path, root)?, "../../../a/b/c");
    Ok(())
}

#[test]
fn trivial_relativize_path() -> Result<(), Box<dyn Error>> {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let relative = Utf8Path::new(".");
    assert_eq!(relativize_path(path, root)?, relative.as_str());
    Ok(())
}

#[test]
#[should_panic]
fn relativize_path_error_relative_path() {
    let path = if cfg!(windows) {
        Utf8Path::new(r"a\b\c")
    } else {
        Utf8Path::new("a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let _unreachable = relativize_path(path, root);
}

#[test]
#[should_panic]
fn relativize_path_error_relative_root() {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"a\b\c")
    } else {
        Utf8Path::new("a/b/c")
    };
    let _unreachable = relativize_path(path, root);
}

#[test]
#[should_panic]
fn relativize_path_error_non_canonical() {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\..\c")
    } else {
        Utf8Path::new("/a/../c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let _unreachable = relativize_path(path, root);
}

#[test]
#[should_panic]
fn relativize_path_error_non_canonical_root() {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\a\..\c")
    } else {
        Utf8Path::new("/a/../c")
    };
    let _unreachable = relativize_path(path, root);
}

#[cfg(target_os = "windows")]
#[test]
fn relativize_path_error_non_common_prefix() -> Result<(), Box<dyn Error>> {
    use crate::paths::RelativizePathError;

    let path = Utf8Path::new(r"C:\a\b\c");
    let root = Utf8Path::new(r"D:\a\b\c");
    let Err(err) = relativize_path(path, root) else {
        panic!("`relativize_path` did not return error");
    };
    let RelativizePathError::NoCommonPrefix {
        path: err_path,
        root: err_root,
    } = err;
    assert_eq!(*err_path, *path);
    assert_eq!(*err_root, *root);
    Ok(())
}

#[cfg(all(unix, feature = "filesystem"))]
#[test]
fn relativize_path_in_stays_inside_the_root() {
    let inside = |path: &str| {
        relativize_path_in(path, "/sysand-no-such-dir/a")
            .unwrap()
            .map(typed_path::Utf8PathBuf::into_string)
    };
    assert_eq!(inside("/sysand-no-such-dir/a/b/c").as_deref(), Some("b/c"));
    assert_eq!(inside("/sysand-no-such-dir/a").as_deref(), Some(""));
    assert_eq!(inside("/sysand-no-such-dir/b"), None);
    // a `..` that survives canonicalization of the existing prefix leaves the root
    assert_eq!(inside("/sysand-no-such-dir/a/../b"), None);
    assert_eq!(inside("/sysand-no-such-dir/a/b/../../.."), None);
}

#[cfg(all(unix, feature = "filesystem"))]
#[test]
fn canonicalize_prefix_returns_errors_other_than_missing() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = camino_tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let result = canonicalize_prefix(locked.join("inner/file"));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();

    // root can read everything
    if std::fs::read_dir(&locked).is_ok() {
        return;
    }
    let err = result.unwrap_err();
    assert_eq!(err.source.kind(), std::io::ErrorKind::PermissionDenied);
}

#[cfg(all(unix, feature = "filesystem"))]
#[test]
fn canonicalize_prefix_rejects_a_path_inside_a_file() {
    let dir = camino_tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    std::fs::write(&file, "").unwrap();
    let err = canonicalize_prefix(file.join("inner")).unwrap_err();
    assert_eq!(err.source.kind(), std::io::ErrorKind::NotADirectory);
}
