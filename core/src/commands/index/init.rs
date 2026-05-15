// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    fs::{self},
    io::{ErrorKind, Write},
};

use camino::Utf8Path;
use thiserror::Error;

use crate::{
    index::{INDEX_FILE_NAME, to_json_string},
    index_utils::IndexJson,
    project::utils::{FsIoError, wrapfs},
};

#[derive(Error, Debug)]
pub enum IndexInitError {
    #[error("`sysand index init` cannot be run on an existing index")]
    AlreadyExists,
    #[error("failed to write {INDEX_FILE_NAME}")]
    WriteError(#[from] Box<FsIoError>),
}

pub fn do_index_init<R: AsRef<Utf8Path>>(index_root: R) -> Result<(), IndexInitError> {
    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} index");
    let index = IndexJson { projects: vec![] };
    let index_str = to_json_string(&index);
    wrapfs::create_dir_all(index_root.as_ref())?;
    let index_path = index_root.as_ref().join(INDEX_FILE_NAME);
    let mut file = fs::File::create_new(&index_path).map_err(|e| match e.kind() {
        ErrorKind::AlreadyExists => IndexInitError::AlreadyExists,
        _ => IndexInitError::WriteError(Box::new(FsIoError::CreateFile(index_path.clone(), e))),
    })?;
    file.write_all(index_str.as_bytes())
        .map_err(|e| IndexInitError::WriteError(Box::new(FsIoError::WriteFile(index_path, e))))?;
    Ok(())
}
