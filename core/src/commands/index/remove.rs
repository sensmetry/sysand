// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::fs::File;

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

use crate::{
    index::{
        INDEX_FILE_NAME, JsonFileError, VERSIONS_FILE_NAME, open_json_file, overwrite_file,
        to_json_string,
    },
    index_utils::{
        IndexJson, ParseIriError, ProjectStatus, VersionEntry, VersionStatus, VersionsJson,
        parse_iri,
    },
    project::utils::{FsIoError, wrapfs},
};

#[derive(Debug, Error)]
pub enum IndexRemoveError {
    #[error("index root directory `{0}` not found")]
    IndexRootNotFound(Utf8PathBuf),
    #[error(
        "directory `{index_root}` is not an index as it doesn't have {INDEX_FILE_NAME} file; make sure you run `sysand index init` in this directory before adding any packages"
    )]
    NotAnIndex {
        index_root: Utf8PathBuf,
        #[source]
        source: Box<FsIoError>,
    },
    #[error("Project {iri} doesn't exist")]
    ProjectNotFound { iri: Box<str> },
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

pub fn do_index_remove<R: AsRef<Utf8Path>, I: AsRef<str>, V: AsRef<str>>(
    index_root: R,
    iri: I,
    version: Option<V>,
) -> Result<(), IndexRemoveError> {
    let index_root = index_root.as_ref();
    if !wrapfs::is_dir(index_root)? {
        return Err(IndexRemoveError::IndexRootNotFound(index_root.into()));
    }
    let index_path = index_root.join(INDEX_FILE_NAME);
    let (mut index_file, mut index_value) = open_json_file::<IndexJson>(&index_path, false)
        .map_err(|e| match e {
            JsonFileError::FileDoesNotExist(e) => IndexRemoveError::NotAnIndex {
                index_root: index_root.into(),
                source: e,
            },
            _ => IndexRemoveError::from(e),
        })?;

    let parsed_iri = parse_iri(iri.as_ref())?;
    let iri = parsed_iri.get_iri();
    let Some(project_entry) = index_value.projects.iter_mut().find(|p| p.iri == iri) else {
        return Err(IndexRemoveError::ProjectNotFound { iri: iri.into() });
    };
    if project_entry.status == ProjectStatus::Removed {
        log::warn!("{iri} is already removed");
    } else {
        project_entry.status = ProjectStatus::Removed;
    }
    let index_str = to_json_string(&index_value);
    let project_path = index_root.join(parsed_iri.get_path());

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
            let mut version_found: bool = false;
            remove_versions(
                &project_path,
                &versions_path,
                &mut versions_file,
                &mut versions_value,
                |v| {
                    if v.version == version {
                        version_found = true;
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
            if version_found {
                Ok(())
            } else {
                Err(IndexRemoveError::VersionNotFound {
                    iri: iri.into(),
                    version: version.into(),
                })
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
            overwrite_file(&mut index_file, &index_path, &index_str)?;
            Ok(())
        }
    }
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
            let version_path = project_path.join(&version_entry.version);
            version_entry.status = VersionStatus::Removed;

            let versions_str = to_json_string(&versions_value);
            overwrite_file(versions_file, versions_path, &versions_str)?;
            wrapfs::remove_dir_all(version_path)?;
        }
    }
    Ok(())
}
