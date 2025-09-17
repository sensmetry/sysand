// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::{Path, PathBuf};

use crate::discover::discover_project;

pub fn do_root<P: AsRef<Path>>(path: P) -> Option<PathBuf> {
    discover_project(path).map(|e| e.root_path())
}
