// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::error::Error;

use camino::Utf8Path;

use crate::project::utils::relativize_path;

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
    let relative = if cfg!(windows) {
        Utf8Path::new(r"a\b\c")
    } else {
        Utf8Path::new("a/b/c")
    };
    assert_eq!(relativize_path(path, root)?, relative.as_str());
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
    let relative = if cfg!(windows) {
        Utf8Path::new(r"..\..\..\a\b\c")
    } else {
        Utf8Path::new("../../../a/b/c")
    };
    assert_eq!(relativize_path(path, root)?, relative.as_str());
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
    let _ = relativize_path(path, root);
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
    let _ = relativize_path(path, root);
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
    let _ = relativize_path(path, root);
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
    let _ = relativize_path(path, root);
}

#[cfg(target_os = "windows")]
#[test]
fn relativize_path_error_non_common_prefix() -> Result<(), Box<dyn Error>> {
    use crate::project::utils::RelativizePathError;

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
