// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino::Utf8PathBuf;
use thiserror::Error;

use crate::{
    env::index::{IndexJson, Status, VersionsJson},
    index::{
        INDEX_FILE_NAME, JsonFileError, NOT_AN_INDEX_MESSAGE, VERSIONS_FILE_NAME, open_json_file,
        overwrite_file, to_json_string,
    },
    index_utils::{ParseIriError, parse_iri},
    project::utils::FsIoError,
};

#[derive(Debug, Error)]
pub enum IndexYankError {
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
    #[error("{iri} version {version} is removed so it cannot be yanked")]
    VersionRemoved { iri: Box<str>, version: String },
    #[error("{iri} version {version} does not exist")]
    VersionNotFound { iri: Box<str>, version: Box<str> },
}

impl From<JsonFileError> for IndexYankError {
    fn from(value: JsonFileError) -> Self {
        match value {
            JsonFileError::FileDoesNotExist(e) => IndexYankError::Io(e),
            JsonFileError::Io(e) => IndexYankError::Io(e),
            JsonFileError::InvalidJsonFile { path, source } => {
                IndexYankError::InvalidJsonFile { path, source }
            }
        }
    }
}

pub fn do_index_yank<I: AsRef<str>, V: AsRef<str>>(
    iri: I,
    version: V,
) -> Result<(), IndexYankError> {
    let index_path = Utf8PathBuf::from(INDEX_FILE_NAME);
    // This is here just to report an error in case this is not an index
    _ = open_json_file::<IndexJson>(&index_path, false).map_err(|e| match e {
        JsonFileError::FileDoesNotExist(e) => IndexYankError::NotAnIndex(e),
        _ => IndexYankError::from(e),
    })?;

    let parsed_iri = parse_iri(iri.as_ref())?;
    let iri = parsed_iri.clone().to_iri();
    let project_path: Utf8PathBuf = parsed_iri.to_path_segments().iter().collect();

    let versions_path = project_path.join(VERSIONS_FILE_NAME);
    let (versions_file, mut versions_value) = open_json_file::<VersionsJson>(&versions_path, true)?;

    let yanking = "Yanking";
    let header = crate::style::get_style_config().header;

    // Specifically don't report any errors if the version is not a valid semver,
    // since if the project with invalid semver got in there somehow, it should
    // be possible to yank
    let version = version.as_ref();
    log::info!("{header}{yanking:>12}{header:#} {iri} version {version}");

    let mut yanked: usize = 0;
    for i in 0..versions_value.versions.len() {
        let version_entry = &mut versions_value.versions[i];
        if version_entry.version == version {
            yanked += 1;
            match version_entry.status {
                Status::Available => {
                    version_entry.status = Status::Yanked;
                    let versions_str = to_json_string(&versions_value);
                    // TODO(JP): ask about this. It's not ideal to re-serialize versions and write the whole
                    // thing to file every time, but I would like to actually remove the files only when
                    // the version is specified as removed
                    overwrite_file(&versions_file, &versions_path, &versions_str)?;
                }
                Status::Yanked => {
                    log::warn!("{iri} version {version} is already yanked")
                }
                Status::Removed => {
                    return Err(IndexYankError::VersionRemoved {
                        iri: iri.into(),
                        version: version.to_string(),
                    });
                }
            }
        }
    }
    match yanked {
        0 => Err(IndexYankError::VersionNotFound {
            iri: iri.into(),
            version: version.into(),
        }),
        1 => Ok(()),
        2.. => {
            log::warn!("{iri} had duplicate versions {version}, all are yanked");
            Ok(())
        }
    }
}
