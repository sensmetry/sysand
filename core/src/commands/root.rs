// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    discover::{Ancestry, Seek},
    project::utils::{FsIoError, wrapfs},
};

/// The canonical root of the nearest project at or above `path`.
pub fn do_root<P: AsRef<Utf8Path>>(path: P) -> Result<Option<Utf8PathBuf>, Box<FsIoError>> {
    Ancestry::walk(path, Seek::ProjectToo)?
        .project()
        .map(wrapfs::canonicalize)
        .transpose()
}
