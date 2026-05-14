// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::fs::File;

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

use crate::{
    index::{
        INDEX_FILE_NAME, JsonFileError, NOT_AN_INDEX_MESSAGE, VERSIONS_FILE_NAME, open_json_file,
        overwrite_file, to_json_string,
    },
    index_utils::{
        IndexJson, ParseIriError, ProjectStatus, VersionEntry, VersionStatus, VersionsJson,
        parse_iri,
    },
    project::utils::{FsIoError, wrapfs},
};

#[derive(Debug, Error)]
pub enum IndexRemoveError {
    #[error("{NOT_AN_INDEX_MESSAGE}")]
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
    #[error("{iri} version {version} does not exist")]
    VersionNotFound { iri: Box<str>, version: Box<str> },
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
    let (mut index_file, mut index_value) = open_json_file::<IndexJson>(&index_path, false)
        .map_err(|e| match e {
            JsonFileError::FileDoesNotExist(e) => IndexRemoveError::NotAnIndex(e),
            _ => IndexRemoveError::from(e),
        })?;

    let parsed_iri = parse_iri(iri.as_ref())?;
    let iri = parsed_iri.clone().to_iri();
    let project_path: Utf8PathBuf = parsed_iri.to_path_segments().iter().collect();

    let versions_path = project_path.join(VERSIONS_FILE_NAME);
    let (mut versions_file, mut versions_value) =
        open_json_file::<VersionsJson>(&versions_path, true)?;

    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    match version {
        Some(version) => {
            // Specifically don't report any errors if the version is not a valid semver,
            // since if the project with invalid semver got in there somehow, it should
            // be possible to remove
            let version = version.as_ref();
            log::info!("{header}{removing:>12}{header:#} {iri} version {version}");
            let mut removed: usize = 0;
            remove_versions(
                &project_path,
                &versions_path,
                &mut versions_file,
                &mut versions_value,
                |v| {
                    if v.version == version {
                        removed += 1;
                        if matches!(v.status, VersionStatus::Removed) {
                            log::warn!("{iri} version {version} is already removed");
                            false
                        } else {
                            true
                        }
                    } else {
                        false
                    }
                },
            )?;
            match removed {
                0 => Err(IndexRemoveError::VersionNotFound {
                    iri: iri.into(),
                    version: version.into(),
                }),
                1 => Ok(()),
                // TODO(JP): this is actually impossible. If the same version appears multiple times
                // in versions.json, upon trying to remove the version directory the second time IO error
                // would be raised
                2.. => {
                    log::warn!("{iri} had duplicate versions {version}, all are removed");
                    Ok(())
                }
            }
        }
        None => {
            log::info!("{header}{removing:>12}{header:#} {iri}");
            remove_versions(
                &project_path,
                &versions_path,
                &mut versions_file,
                &mut versions_value,
                |v| !matches!(v.status, VersionStatus::Removed),
            )?;
            // TODO(JP) report a warning if no such project was removed or if more than one was removed
            for project in index_value.projects.iter_mut() {
                if project.iri == iri {
                    project.status = ProjectStatus::Removed;
                }
            }
            let index_str = to_json_string(&index_value);
            overwrite_file(&mut index_file, &index_path, &index_str)?;
            Ok(())
        }
    }
}

fn remove_versions<F: FnMut(&VersionEntry) -> bool>(
    project_path: &Utf8Path,
    versions_path: &Utf8Path,
    versions_file: &mut File,
    versions_value: &mut VersionsJson,
    mut if_remove_version: F,
) -> Result<(), IndexRemoveError> {
    for i in 0..versions_value.versions.len() {
        let version_entry = &mut versions_value.versions[i];
        if if_remove_version(version_entry) {
            // TODO(JP) should if the version is absent from versions.json but the files do exist, should probably remove them anyway
            let version_path = project_path.join(&version_entry.version);
            version_entry.status = VersionStatus::Removed;

            let versions_str = to_json_string(&versions_value);
            // TODO(JP): ask about this. It's not ideal to re-serialize versions and write the whole
            // thing to file every time, but I would like to actually remove the files only when
            // the version is specified as removed
            overwrite_file(versions_file, versions_path, &versions_str)?;
            wrapfs::remove_dir_all(version_path)?;
        }
    }
    Ok(())
}
