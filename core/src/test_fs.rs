// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Filesystem checks for tests. `Path::{exists, is_file, is_dir}` turn I/O errors into
//! `false`, so tests call these, which fail on any I/O error.

use std::path::Path;

use camino::Utf8Path;

use crate::project::utils::wrapfs;

/// Whether `path` exists. Panics if that cannot be determined.
pub fn exists(path: impl AsRef<Path>) -> bool {
    path.as_ref().try_exists().unwrap()
}

/// Whether `path` is a regular file. Panics on I/O errors other than not found.
pub fn is_file(path: impl AsRef<Path>) -> bool {
    wrapfs::is_file(utf8(path.as_ref())).unwrap()
}

/// Whether `path` is a directory. Panics on I/O errors other than not found.
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    wrapfs::is_dir(utf8(path.as_ref())).unwrap()
}

fn utf8(path: &Path) -> &Utf8Path {
    Utf8Path::from_path(path).expect("test paths are UTF-8")
}
