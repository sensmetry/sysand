// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::fs::File;

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

use crate::{
    env::index::{IndexJson, ProjectStatus, Status, VersionsJson},
    index::{
        INDEX_FILE_NAME, JsonFileError, VERSIONS_FILE_NAME, open_json_file, overwrite_file,
        to_json_string,
    },
    index_utils::{ParseIriError, parse_iri},
    project::utils::{FsIoError, wrapfs},
};

#[derive(Debug, Error)]
pub enum IndexRemoveError {
    #[error(
        "current directory is not an index as it doesn't have {INDEX_FILE_NAME} file; make sure you run `sysand index init` in this directory before adding any packages"
    )]
    NotAnIndex(#[source] Box<FsIoError>),
    // TODO(JP): might want to make these more specific
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("patching json `{path}` failed as the current contents are invalid")]
    InvalidJsonFile {
        path: Box<str>,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    InvalidIri(#[from] ParseIriError),
}

impl From<JsonFileError> for IndexRemoveError {
    fn from(value: JsonFileError) -> Self {
        match value {
            JsonFileError::FileDoesNotExist(e) => IndexRemoveError::Io(e),
            JsonFileError::Io(e) => IndexRemoveError::Io(e),
            JsonFileError::InvalidJsonFile { path, source } => {
                IndexRemoveError::InvalidJsonFile { path, source }
            }
        }
    }
}

pub fn do_index_remove<I: AsRef<str>, V: AsRef<str>>(
    iri: I,
    version: Option<V>,
) -> Result<(), IndexRemoveError> {
    let index_path = Utf8PathBuf::from(INDEX_FILE_NAME);
    let (index_file, mut index_value) = open_json_file::<_, IndexJson>(&index_path, false)
        .map_err(|e| match e {
            JsonFileError::FileDoesNotExist(e) => IndexRemoveError::NotAnIndex(e),
            _ => IndexRemoveError::from(e),
        })?;

    let parsed_iri = parse_iri(iri.as_ref())?;
    let iri = parsed_iri.clone().to_iri();
    let project_path: Utf8PathBuf = parsed_iri.to_path_segments().iter().collect();

    let versions_path = project_path.join(VERSIONS_FILE_NAME);
    let (versions_file, mut versions_value) =
        open_json_file::<_, VersionsJson>(&versions_path, true)?;

    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    match version {
        Some(version) => {
            let version = version.as_ref();
            log::info!("{header}{removing:>12}{header:#} {iri} version {version}");
            let _removed = remove_versions(
                &project_path,
                &versions_file,
                &versions_path,
                &mut versions_value,
                |v| v == version,
            )?;
            // TODO(JP) report a warning if no such version was removed or if more than one was removed
        }
        None => {
            log::info!("{header}{removing:>12}{header:#} {iri}");
            _ = remove_versions(
                &project_path,
                &versions_file,
                &versions_path,
                &mut versions_value,
                |_| true,
            )?;
            // TODO(JP) report a warning if no such project was removed or if more than one was removed
            for project in index_value.projects.iter_mut() {
                if project.iri == iri {
                    project.status = ProjectStatus::Removed;
                }
            }
            let index_str = to_json_string(&index_value);
            overwrite_file(&index_file, &index_path, &index_str)?;
        }
    }
    Ok(())
}

fn remove_versions<F: Fn(&str) -> bool>(
    project_path: &Utf8Path,
    versions_file: &File,
    versions_path: &Utf8Path,
    versions_value: &mut VersionsJson,
    if_remove_version: F,
) -> Result<usize, IndexRemoveError> {
    // Specifically don't report any errors if the version is not a valid semver,
    // since if the project with invalid semver got in there somehow, it should
    // be possible to remove
    let mut removed = 0;
    for i in 0..versions_value.versions.len() {
        let version_entry = &mut versions_value.versions[i];
        if !matches!(version_entry.status, Status::Removed)
            && if_remove_version(&version_entry.version)
        {
            // TODO(JP) should if the version is absent from versions.json but the files do exist, should probably remove them anyway
            let version_path = project_path.join(&version_entry.version);
            version_entry.status = Status::Removed;

            let versions_str = to_json_string(&versions_value);
            // TODO(JP): ask about this. It's not ideal to re-serialize versions and write the whole
            // thing to file every time, but I would like to actually remove the files only when
            // the version is specified as removed
            overwrite_file(versions_file, versions_path, &versions_str)?;
            wrapfs::remove_dir_all(version_path)?;
            removed += 1;
        }
    }
    Ok(removed)
}
