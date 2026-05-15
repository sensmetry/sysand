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
    project::utils::FsIoError,
};

#[derive(Error, Debug)]
pub enum IndexInitError {
    #[error("`sysand index init` cannot be run on an existing index")]
    AlreadyExists,
    #[error("failed to write {INDEX_FILE_NAME}")]
    WriteError(#[from] Box<FsIoError>),
}

// impl From<FsIoError> for IndexInitError {
//     fn from(v: FsIoError) -> Self {
//         IndexInitError::WriteError(Box::new(v))
//     }
// }

pub fn do_index_init<R: AsRef<Utf8Path>>(index_root: R) -> Result<(), IndexInitError> {
    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} index");
    let index = IndexJson { projects: vec![] };
    let index_str = to_json_string(&index);
    let index_path = index_root.as_ref().join(INDEX_FILE_NAME);
    // TODO(JP): ask to review this
    let mut file = fs::File::create_new(&index_path).map_err(|err| match err.kind() {
        ErrorKind::AlreadyExists => IndexInitError::AlreadyExists,
        _ => IndexInitError::WriteError(Box::new(FsIoError::CreateFile(index_path.clone(), err))),
    })?;
    file.write(index_str.as_bytes()).map_err(|err| {
        IndexInitError::WriteError(Box::new(FsIoError::WriteFile(index_path, err)))
    })?;
    Ok(())
}

// pub fn do_index_init() -> Result<(), IndexInitError> {
//     let creating = "Creating";
//     let header = crate::style::get_style_config().header;
//     log::info!("{header}{creating:>12}{header:#} index");
//     let index = IndexJson { projects: vec![] };
//     let index_serialized = serde_json::to_string(&index).map_err(serde_json::Error::from)?;
//     let index_path = Utf8PathBuf::from(INDEX_PATH);
//     if wrapfs::is_file(&index_path)? {
//         return Err(IndexInitError::AlreadyExists);
//     }
//     wrapfs::write(&index_path, index_serialized.as_bytes())?;
//     Ok(())
// }
