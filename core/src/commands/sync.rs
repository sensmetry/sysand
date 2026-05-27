// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, num::NonZeroU64};

use thiserror::Error;
use typed_path::Utf8UnixPathBuf;

use crate::{
    commands::env::do_env_install_project,
    env::{ProjectChecksumResult, ReadEnvironment, WriteEnvironment, utils::ErrorBound},
    iri_normalize::canonicalize_iri_tolerant,
    lock::{Lock, Source},
    project::{ProjectChecksum, ProjectRead, memory::InMemoryProject},
};

#[derive(Error, Debug)]
pub enum SyncError<UrlParseError: ErrorBound, GitError: ErrorBound> {
    #[error(
        "incorrect checksum for project with IRI `{iri}` in lockfile:\n\
        expected `{expected}`, but the actual is\n\
        `{actual}`"
    )]
    BadChecksum {
        iri: String,
        expected: ProjectChecksum,
        actual: ProjectChecksum,
    },
    #[error("project with IRI `{0}` is missing `.project.json` or `.meta.json`")]
    BadProject(String),
    #[error("project with IRI(s) {0:?} has no known sources in lockfile")]
    MissingSource(Box<[String]>),
    #[error("no IRI given for project with src_path = `{0}` in lockfile")]
    MissingIriSrcPath(Box<str>),
    #[error("no IRI given for project with remote_src = `{0}` in lockfile")]
    MissingIriRemoteSrc(Box<str>),
    #[error("no IRI given for project with kpar_path = `{0}` in lockfile")]
    MissingIriLocalKparPath(Box<str>),
    #[error("no IRI given for project with remote_kpar = `{0}` in lockfile")]
    MissingIriRemoteKparPath(Box<str>),
    #[error("no IRI given for project with index_kpar = `{0}` in lockfile")]
    MissingIriIndexKparUrl(Box<str>),
    #[error("no IRI given for project with remote_git = `{0}` in lockfile")]
    MissingIriRemoteGitUrl(Box<str>),
    #[error(
        "cannot handle project with IRI `{0}` residing in local file (type `local_src`) storage"
    )]
    MissingSrcPathStorage(Box<str>),
    #[error("cannot handle project with IRI `{0}` residing in remote (type `remote_src`) storage")]
    MissingRemoteSrcStorage(Box<str>),
    #[error(
        "cannot handle project with IRI `{0}` residing in local kpar (type `local_kpar`) storage"
    )]
    MissingLocalKparStorage(Box<str>),
    #[error(
        "cannot handle project with IRI `{0}` residing in remote kpar (type `remote_kpar`) storage"
    )]
    MissingRemoteKparStorage(Box<str>),
    #[error(
        "cannot handle project with IRI `{0}` residing in index kpar (type `index_kpar`) storage"
    )]
    MissingIndexKparStorage(Box<str>),
    #[error(
        "cannot handle project with IRI `{0}` residing in remote git repo (type `remote_git`) storage"
    )]
    MissingRemoteGitStorage(Box<str>),
    #[error("failed to download git project from {0}: {1}")]
    GitDownload(Box<str>, GitError),
    #[error("invalid remote source URL `{0}`:\n{1}")]
    InvalidRemoteSource(Box<str>, UrlParseError),
    #[error("no supported sources for project with IRI `{0}`")]
    UnsupportedSources(String),
    #[error("failed to install project `{uri}`:\n{cause}")]
    InstallFail { uri: Box<str>, cause: String },
    #[error(
        "tried to install a non-provided version {version} of `{iri}`, which is\n\
        an IRI marked as being provided by your tooling; provided versions are:\n\
        {provided_versions:?}"
    )]
    InvalidProvidedVersion {
        iri: Box<str>,
        version: Box<str>,
        provided_versions: Vec<String>,
    },
    // TODO: preserve error type
    #[error("project read error: {0}")]
    ProjectRead(String),
}

// TODO: take `lock` by value
// TODO: Use AnyProject::try_from_source to avoid having so many arguments
#[allow(clippy::too_many_arguments)]
pub fn do_sync<
    Environment,
    CreateSrcPathStorage,
    SrcPathStorage,
    CreateRemoteSrcStorage,
    RemoteSrcStorage,
    CreateKParPathStorage,
    KParPathStorage,
    CreateRemoteKParStorage,
    RemoteKParStorage,
    CreateIndexKParStorage,
    IndexKParStorage,
    UrlParseError: ErrorBound,
    CreateRemoteGitStorage,
    RemoteGitStorage,
    GitError: ErrorBound,
>(
    lockfile: &Lock,
    env: &mut Environment,
    src_path_storage: Option<CreateSrcPathStorage>,
    remote_src_storage: Option<CreateRemoteSrcStorage>,
    kpar_path_storage: Option<CreateKParPathStorage>,
    remote_kpar_storage: Option<CreateRemoteKParStorage>,
    index_kpar_storage: Option<CreateIndexKParStorage>,
    remote_git_storage: Option<CreateRemoteGitStorage>,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
) -> Result<(), SyncError<UrlParseError, GitError>>
where
    Environment: ReadEnvironment + WriteEnvironment,
    CreateSrcPathStorage: Fn(Utf8UnixPathBuf, String) -> SrcPathStorage,
    SrcPathStorage: ProjectRead,
    CreateRemoteSrcStorage: Fn(String, String) -> Result<RemoteSrcStorage, UrlParseError>,
    RemoteSrcStorage: ProjectRead,
    CreateKParPathStorage: Fn(String, NonZeroU64, String) -> KParPathStorage,
    KParPathStorage: ProjectRead,
    CreateRemoteKParStorage:
        Fn(String, NonZeroU64, String) -> Result<RemoteKParStorage, UrlParseError>,
    RemoteKParStorage: ProjectRead,
    CreateIndexKParStorage:
        Fn(String, NonZeroU64, String) -> Result<IndexKParStorage, UrlParseError>,
    IndexKParStorage: ProjectRead,
    CreateRemoteGitStorage: Fn(String) -> Result<RemoteGitStorage, GitError>,
    RemoteGitStorage: ProjectRead,
{
    let syncing = "Syncing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{syncing:>12}{header:#} env");

    // Do `continue 'main_loop` if it becomes clear that no env changes will be made
    // for the current iteration `project`
    let mut updated = false;
    'main_loop: for project in lockfile.projects.iter() {
        // TODO: We need a proper way to treat multiple IRIs here
        let main_uri = project.identifiers.first();

        for iri in &project.identifiers {
            let excluded_versions = if let Ok(parsed_iri) = fluent_uri::Iri::parse(iri.clone()) {
                // TODO: maybe canonicalize on lock read, or don't canonicalize at all?
                provided_iris.get(canonicalize_iri_tolerant(parsed_iri.borrow()).as_str())
            } else {
                provided_iris.get(iri.as_str())
            };

            if let Some(versions) = excluded_versions {
                let mut provided_versions = vec![];

                for project_version in versions {
                    // Provided projects must have complete metadata
                    let version = project_version.version().unwrap().unwrap();
                    if project.version == version {
                        log::debug!("`{iri}` is marked as provided, skipping installation");
                        continue 'main_loop;
                    }

                    provided_versions.push(version);
                }

                return Err(SyncError::InvalidProvidedVersion {
                    iri: iri.as_str().into(),
                    version: project.version.as_str().into(),
                    provided_versions,
                });
            }
        }

        if project.sources.is_empty() {
            return Err(SyncError::MissingSource(
                project.identifiers.as_slice().into(),
            ));
        }

        for iri in &project.identifiers {
            // TODO: move functionality to check if any of a set of IRIs is installed to env trait
            for source in &project.sources {
                if let Some(checksum) = source.to_checksum()
                    && env
                        .has_version_verified(iri, &project.version, &checksum)
                        .map_err(|e| SyncError::ProjectRead(e.to_string()))?
                        == ProjectChecksumResult::Match
                {
                    log::debug!("`{iri}` found in .sysand");
                    continue 'main_loop;
                }
            }
        }

        let mut no_supported = true;
        // TODO: does it make sense to install the same project from all the sources?
        for source in &project.sources {
            let supported = true;
            match source {
                Source::Editable { editable } => {
                    // Nothing to install for editable
                    log::debug!("skipping installation of editable project from `{editable}`");
                    continue 'main_loop;
                }
                Source::LocalSrc { src_path, checksum } => {
                    let uri = main_uri
                        .ok_or_else(|| SyncError::MissingIriSrcPath(src_path.as_str().into()))?;
                    let src_path_storage = src_path_storage
                        .as_ref()
                        .ok_or_else(|| SyncError::MissingSrcPathStorage(uri.as_str().into()))?;
                    let storage = src_path_storage(src_path.clone(), checksum.clone());
                    log::debug!("trying to install `{uri}` from src_path `{src_path}`");
                    try_install(
                        uri,
                        &project.version,
                        // TODO: avoid clone
                        &ProjectChecksum::Project(checksum.to_owned()),
                        storage,
                        env,
                    )?;
                }
                Source::RemoteSrc {
                    remote_src,
                    checksum,
                } => {
                    let uri = main_uri.as_ref().ok_or_else(|| {
                        SyncError::MissingIriRemoteSrc(remote_src.as_str().into())
                    })?;
                    let remote_src_storage = remote_src_storage
                        .as_ref()
                        .ok_or_else(|| SyncError::MissingRemoteSrcStorage(uri.as_str().into()))?;
                    let storage = remote_src_storage(remote_src.clone(), checksum.clone())
                        .map_err(|e| {
                            SyncError::InvalidRemoteSource(remote_src.as_str().into(), e)
                        })?;
                    log::debug!("trying to install `{uri}` from remote_src: {remote_src}");
                    try_install(
                        uri,
                        &project.version,
                        &ProjectChecksum::Project(checksum.to_owned()),
                        storage,
                        env,
                    )?;
                }
                Source::LocalKpar {
                    kpar_path,
                    kpar_size,
                    kpar_digest,
                } => {
                    let uri = main_uri.ok_or_else(|| {
                        SyncError::MissingIriLocalKparPath(kpar_path.as_str().into())
                    })?;
                    let kpar_path_storage = kpar_path_storage.as_ref().ok_or_else(|| {
                        SyncError::MissingLocalKparStorage(kpar_path.as_str().into())
                    })?;
                    let storage = kpar_path_storage(
                        kpar_path.as_str().to_owned(),
                        *kpar_size,
                        kpar_digest.to_owned(),
                    );
                    log::debug!("trying to install `{uri}` from kpar_path: {kpar_path}");
                    try_install(
                        uri,
                        &project.version,
                        &ProjectChecksum::Kpar(kpar_digest.to_owned()),
                        storage,
                        env,
                    )?;
                }
                Source::RemoteKpar {
                    remote_kpar,
                    kpar_size,
                    kpar_digest,
                } => {
                    let uri = main_uri.ok_or_else(|| {
                        SyncError::MissingIriRemoteKparPath(remote_kpar.as_str().into())
                    })?;
                    let remote_kpar_storage = remote_kpar_storage.as_ref().ok_or_else(|| {
                        SyncError::MissingRemoteKparStorage(remote_kpar.as_str().into())
                    })?;
                    let storage = remote_kpar_storage(
                        remote_kpar.to_owned(),
                        *kpar_size,
                        kpar_digest.to_owned(),
                    )
                    .map_err(|e| SyncError::InvalidRemoteSource(remote_kpar.as_str().into(), e))?;
                    log::debug!("trying to install `{uri}` from remote_kpar: {remote_kpar}");
                    try_install(
                        uri,
                        &project.version,
                        &ProjectChecksum::Kpar(kpar_digest.to_owned()),
                        storage,
                        env,
                    )?;
                }
                Source::IndexKpar {
                    index_kpar,
                    kpar_size,
                    kpar_digest,
                } => {
                    let uri = main_uri.ok_or_else(|| {
                        SyncError::MissingIriIndexKparUrl(index_kpar.as_str().into())
                    })?;
                    let index_kpar_storage = index_kpar_storage
                        .as_ref()
                        .ok_or_else(|| SyncError::MissingIndexKparStorage(uri.as_str().into()))?;
                    let storage =
                        index_kpar_storage(index_kpar.to_owned(), *kpar_size, kpar_digest.clone())
                            .map_err(|e| {
                                SyncError::InvalidRemoteSource(index_kpar.as_str().into(), e)
                            })?;
                    log::debug!("trying to install `{uri}` from index_kpar: {index_kpar}");
                    try_install(
                        uri,
                        &project.version,
                        &ProjectChecksum::Kpar(kpar_digest.to_owned()),
                        storage,
                        env,
                    )?;
                }
                // TODO: git is for now assumed to be editable; in particular we should probably set
                // editable=true in lockfile/env.toml or some other indicator that the project is expected to change and no
                // integrity checking will be done
                // To avoid having remote URLs for editable projects in env.toml, for now on sync unconditionally
                // install the project
                Source::RemoteGit { remote_git } => {
                    let uri = main_uri.ok_or_else(|| {
                        SyncError::MissingIriRemoteGitUrl(remote_git.as_str().into())
                    })?;
                    let remote_git_storage = remote_git_storage.as_ref().ok_or_else(|| {
                        SyncError::MissingRemoteGitStorage(remote_git.as_str().into())
                    })?;
                    let storage = remote_git_storage(remote_git.clone())
                        .map_err(|e| SyncError::GitDownload(remote_git.as_str().into(), e))?;
                    log::debug!("trying to install `{uri}` from remote_git: {remote_git}");
                    do_env_install_project(uri, &project.version, &storage, None, env, true, true)
                        .map_err(|e| SyncError::InstallFail {
                            uri: uri.as_str().into(),
                            cause: e.to_string(),
                        })?;
                }
            }
            if supported {
                no_supported = false;
            }
        }
        if no_supported {
            return Err(SyncError::UnsupportedSources(
                main_uri
                    .cloned()
                    .unwrap_or_else(|| "project without IRI".to_string()),
            ));
        }
        updated = true;
    }
    if !updated {
        log::info!("{:>12} nothing to do: env is already up to date", ' ');
    }
    Ok(())
}

fn try_install<
    E: ReadEnvironment + WriteEnvironment,
    P: ProjectRead,
    U: ErrorBound,
    G: ErrorBound,
    S: AsRef<str>,
>(
    uri: S,
    version: &str,
    expected_checksum: &ProjectChecksum,
    storage: P,
    env: &mut E,
) -> Result<(), SyncError<U, G>> {
    let uri = uri.as_ref();
    let actual_checksum = storage
        .checksum_canonical_variant()
        .map_err(|e| SyncError::ProjectRead(e.to_string()))?;
    if expected_checksum == &actual_checksum {
        // TODO: Need to decide how to handle existing installations and possible flags to modify behavior
        do_env_install_project(
            uri,
            version,
            &storage,
            Some(actual_checksum),
            env,
            true,
            true,
        )
        .map_err(|e| SyncError::InstallFail {
            uri: uri.into(),
            cause: e.to_string(),
        })?;
    } else {
        return Err(SyncError::BadChecksum {
            iri: uri.into(),
            expected: expected_checksum.to_owned(),
            actual: actual_checksum,
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "./sync_tests.rs"]
mod tests;
