// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    fs::{self},
    io::{ErrorKind, Write as _},
};

use camino::Utf8Path;
use thiserror::Error;

use super::{INDEX_CONFIG_FILE_NAME, INDEX_FILE_NAME};

use crate::{
    index::model::IndexJson,
    project::utils::{FsIoError, wrapfs},
    utils::to_pretty_json_string,
};

#[derive(Error, Debug)]
pub enum IndexInitError {
    #[error("`sysand index init` cannot be run on an existing index")]
    AlreadyExists,
    #[error("failed to write {INDEX_FILE_NAME}")]
    WriteError(#[from] Box<FsIoError>),
}

/// Create `name` under `index_root` with `content`, refusing to replace
/// an existing file.
fn create_new(index_root: &Utf8Path, name: &str, content: &str) -> Result<(), IndexInitError> {
    let path = index_root.join(name);
    let mut file = fs::File::create_new(&path).map_err(|e| match e.kind() {
        ErrorKind::AlreadyExists => IndexInitError::AlreadyExists,
        _ => IndexInitError::WriteError(Box::new(FsIoError::CreateFile(path.clone(), e))),
    })?;
    file.write_all(content.as_bytes())
        .map_err(|e| IndexInitError::WriteError(Box::new(FsIoError::WriteFile(path, e))))
}

pub fn do_index_init<R: AsRef<Utf8Path>>(index_root: R) -> Result<(), IndexInitError> {
    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} index");
    let index = IndexJson { projects: vec![] };
    let index_str = to_pretty_json_string(&index);
    wrapfs::create_dir_all(index_root.as_ref())?;
    // The discovery document first, so a directory that has `index.json`
    // always has it too: `{}` is the document whose every field defaults
    // (`index_root` to the location it was fetched from), which lets the
    // same directory be read from disk now and served over HTTP later
    // and be discovered as the same index (design/index-protocol.md
    // "Filesystem indexes").
    create_new(index_root.as_ref(), INDEX_CONFIG_FILE_NAME, "{}\n")?;
    create_new(index_root.as_ref(), INDEX_FILE_NAME, &index_str)?;
    Ok(())
}
