// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::{Utf8Path, Utf8PathBuf};

use crate::discover::discover_project;

pub fn do_root<P: AsRef<Utf8Path>>(path: P) -> Option<Utf8PathBuf> {
    discover_project(path).map(|e| e.root_path())
}
