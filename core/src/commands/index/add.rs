// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{cmp::Reverse, collections::HashMap, num::NonZero, str::FromStr};

use camino::{Utf8Path, Utf8PathBuf};
use semver::Version;
use thiserror::Error;

use crate::{
    index::{
        INDEX_FILE_NAME, JsonFileError, KPAR_FILE_NAME, META_FILE_NAME, VERSIONS_FILE_NAME,
        open_json_file, overwrite_file, to_json_string,
    },
    index_utils::{
        IndexJson, IndexProject, ParseIriError, ParsedIri, ProjectStatus, VersionEntry,
        VersionStatus, VersionsJson, parse_iri,
    },
    project::{
        CanonicalizationError, ProjectRead as _,
        local_kpar::{LocalKParError, LocalKParProject},
        utils::{FsIoError, wrapfs},
    },
    purl::{is_valid_unnormalized_name, is_valid_unnormalized_publisher},
};

#[derive(Error, Debug)]
pub enum IndexAddError {
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
    #[error(".project.json file is missing from KPAR `{0}`")]
    MissingInfo(Utf8PathBuf),
    #[error(".meta.json file is missing from the KPAR `{0}`")]
    MissingMeta(Utf8PathBuf),
    #[error("failed to compute project digest")]
    ProjectDigest(#[from] CanonicalizationError<LocalKParError>),
    #[error(transparent)]
    ProjectRead(#[from] LocalKParError),
    #[error(transparent)]
    InvalidIri(#[from] ParseIriError),
    #[error("invalid publisher in .project.json")]
    InvalidPublisherInProject,
    #[error("invalid name in .project.json")]
    InvalidNameInProject,
    #[error(
        "unable to construct project path, for that either .project.json needs to specify publisher, or iri needs to be provided"
    )]
    MissingPublisherAndIri,
    #[error(".meta.json in KPAR {kpar_path} contains invalid semantic version {version}")]
    InvalidKparVersion {
        version: Box<str>,
        kpar_path: Utf8PathBuf,
        #[source]
        source: semver::Error,
    },
    #[error("project {iri} is removed so no new version can be added")]
    ProjectRemoved { iri: Box<str> },
    #[error("two projects with iri {iri} found in {INDEX_FILE_NAME}")]
    DuplicateProject { iri: Box<str> },
    #[error("`{versions_path}` contains invalid semantic version {version}")]
    InvalidExistingVersion {
        version: String,
        versions_path: Utf8PathBuf,
        #[source]
        source: semver::Error,
    },
    #[error("file `{path} contains duplicate version {version}")]
    DuplicateVersion { version: String, path: Utf8PathBuf },
    // TODO(JP): include the iri of the project here and look through other errors if need to do the same
    #[error("{iri} version {version} already exists")]
    VersionAlreadyExists { iri: Box<str>, version: Version },
    #[error(
        "{iri} version {version} is yanked so it cannot be added again; yanked version can only stay yanked or be removed"
    )]
    VersionYanked { iri: Box<str>, version: Version },
    #[error(
        "{iri} version {version} is removed so it cannot be added again; removed version can only stay removed"
    )]
    VersionRemoved { iri: Box<str>, version: Version },
}

impl From<JsonFileError> for IndexAddError {
    fn from(value: JsonFileError) -> Self {
        match value {
            JsonFileError::FileDoesNotExist(e) => IndexAddError::Io(e),
            JsonFileError::Io(e) => IndexAddError::Io(e),
            JsonFileError::InvalidJsonFile { path, source } => {
                IndexAddError::InvalidJsonFile { path, source }
            }
        }
    }
}

// TODO(JP): Ideally the same method would specify that digest is Sha256 and add sha256 prefix
fn to_explicit_digest(digest: String) -> String {
    format!("sha256:{digest}")
}

pub fn do_index_add<R: AsRef<Utf8Path>, P: AsRef<Utf8Path>, I: AsRef<str>>(
    index_root: R,
    kpar_path: P,
    // The type is str, not Iri so that a better error can be reported in some cases
    // for example when the publisher contains a space
    iri: Option<I>,
) -> Result<(), IndexAddError> {
    let index_root = index_root.as_ref();
    let index_path = index_root.join(INDEX_FILE_NAME);
    let (mut index_file, mut index_value) = open_json_file::<IndexJson>(&index_path, false)
        .map_err(|e| match e {
            JsonFileError::FileDoesNotExist(e) => IndexAddError::NotAnIndex(e),
            _ => IndexAddError::from(e),
        })?;

    let kpar_path_abs = wrapfs::absolute(&kpar_path)?;
    // TODO(JP)(review): do we want to allow root to be in non-standard place?
    let local_project =
        LocalKParProject::new_guess_root(&kpar_path_abs).map_err(LocalKParError::Io)?;
    let Some(info) = local_project.get_info()? else {
        return Err(IndexAddError::MissingInfo(kpar_path_abs.clone()));
    };
    let Some(meta) = local_project.get_meta()? else {
        return Err(IndexAddError::MissingMeta(kpar_path_abs));
    };
    let project_digest = to_explicit_digest(
        local_project
            .checksum_canonical_hex()?
            .expect("This should only be None when .project.json or .meta.json is missing"),
    );

    let parsed_iri = match (iri, &info.publisher) {
        (Some(iri), _) => {
            parse_iri(iri.as_ref())?
            // TODO(JP) ensure the project name (and publisher if specified) match the IRI if normalized
        }
        (None, Some(publisher)) => {
            if !is_valid_unnormalized_publisher(publisher) {
                return Err(IndexAddError::InvalidPublisherInProject);
            }
            if !is_valid_unnormalized_name(&info.name) {
                return Err(IndexAddError::InvalidNameInProject);
            }
            ParsedIri::Sysand {
                publisher: publisher.clone(),
                name: info.name.clone(),
            }
        }
        (None, None) => {
            return Err(IndexAddError::MissingPublisherAndIri);
        }
    };

    let iri = parsed_iri.get_iri();
    let project_path = index_root.join(parsed_iri.get_path());

    let project_entries: Vec<_> = index_value
        .projects
        .iter()
        .filter(|p| p.iri == iri)
        .collect();
    let is_project_new = match project_entries[..] {
        [] => {
            index_value.projects.push(IndexProject {
                iri: iri.to_string(),
                status: ProjectStatus::Available,
            });
            true
        }
        [project_entry] => match project_entry.status {
            ProjectStatus::Available => false,
            ProjectStatus::Removed => {
                return Err(IndexAddError::ProjectRemoved { iri: iri.into() });
            }
        },
        [_, _, ..] => return Err(IndexAddError::DuplicateProject { iri: iri.into() }),
    };

    let version: &str = &info.version;
    let semver = Version::from_str(version).map_err(|e| IndexAddError::InvalidKparVersion {
        version: version.into(),
        kpar_path: kpar_path.as_ref().into(),
        source: e,
    })?;

    let info_str = to_json_string(&info);
    let meta_str = to_json_string(&meta);

    wrapfs::create_dir_all(&project_path)?;

    let versions_path = project_path.join(VERSIONS_FILE_NAME);
    let (mut versions_file, mut versions_value) =
        open_json_file::<VersionsJson>(&versions_path, true)?;

    // Use Reverse  so that the highest versions go first when
    let str_to_semver: HashMap<String, Reverse<Version>> = versions_value
        .versions
        .iter()
        .map(|v| {
            println!("Version {}", v.version);
            match Version::from_str(&v.version) {
                Ok(other_semver) => Ok((v.version.clone(), Reverse(other_semver))),
                Err(e) => Err(IndexAddError::InvalidExistingVersion {
                    version: v.version.clone(),
                    versions_path: versions_path.clone(),
                    source: e,
                }),
            }
        })
        .collect::<Result<_, _>>()?;
    let version_key = |v: &VersionEntry| str_to_semver.get(&v.version).unwrap();

    versions_value.versions.sort_by_key(version_key);

    for [ver_entry1, ver_entry2] in versions_value.versions.array_windows() {
        if ver_entry1.version == ver_entry2.version {
            // Strictly speaking this is unnecessary for adding the new project
            // but still good to check
            return Err(IndexAddError::DuplicateVersion {
                version: ver_entry1.version.clone(),
                path: versions_path,
            });
        }
    }

    let insert_ind = match versions_value
        .versions
        .binary_search_by_key(&&Reverse(semver.clone()), version_key)
    {
        Ok(ind) => {
            return Err(match versions_value.versions[ind].status {
                VersionStatus::Available => IndexAddError::VersionAlreadyExists {
                    iri: iri.into(),
                    version: semver.clone(),
                },
                VersionStatus::Yanked => IndexAddError::VersionYanked {
                    iri: iri.into(),
                    version: semver.clone(),
                },
                VersionStatus::Removed => IndexAddError::VersionRemoved {
                    iri: iri.into(),
                    version: semver.clone(),
                },
            });
        }
        Err(ind) => ind,
    };
    versions_value.versions.insert(
        insert_ind,
        VersionEntry {
            version: version.to_string(),
            usage: info.usage,
            project_digest,
            // The zip file does contain .project.json and .meta.json at this point
            // so it cannot be empty
            kpar_size: NonZero::new(local_project.file_size()?).unwrap(),
            kpar_digest: to_explicit_digest(local_project.digest_sha256()?),
            status: VersionStatus::Available,
        },
    );

    let versions_str = to_json_string(&versions_value);
    let index_str = to_json_string(&index_value);

    let adding = "Adding";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{adding:>12}{header:#} {iri} version {version}");

    let version_path = project_path.join(version);
    wrapfs::create_dir(&version_path)?;

    // TODO(JP): probably want to nuke the version dir if any of these fail
    wrapfs::copy(kpar_path, version_path.join(KPAR_FILE_NAME))?;
    wrapfs::write(version_path.join(INDEX_FILE_NAME), info_str)?;
    wrapfs::write(version_path.join(META_FILE_NAME), meta_str)?;

    overwrite_file(&mut versions_file, &versions_path, &versions_str)?;
    if is_project_new {
        overwrite_file(&mut index_file, &index_path, &index_str)?;
    }

    Ok(())
}
