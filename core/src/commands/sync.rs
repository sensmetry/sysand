// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;

use thiserror::Error;

use crate::{
    commands::env::do_env_install_project,
    env::{ReadEnvironment, WriteEnvironment},
    lock::{Lock, Source},
    project::{ProjectRead, memory::InMemoryProject},
};

#[derive(Error, Debug)]
pub enum SyncError<UrlParseError> {
    #[error("incorrect checksum for project with IRI `{0}` in lockfile")]
    BadChecksum(String),
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
    #[error("invalid remote source URL `{0}`:\n{1}")]
    InvalidRemoteSource(Box<str>, UrlParseError),
    #[error("no supported sources for project with IRI `{0}`")]
    UnsupportedSources(String),
    #[error("failed to install project `{uri}`:\n{cause}")]
    InstallFail { uri: Box<str>, cause: String },
    #[error(
        "tried to install a non-provided version (checksum {hash}) of `{iri}`, which is an IRI marked as being provided by your tooling"
    )]
    InvalidProvidedVersion {
        iri: Box<str>,
        hash: Box<str>,
        provided: Vec<String>,
    },
    #[error("project read error: {0}")]
    ProjectRead(String),
}

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
    UrlParseError,
>(
    lockfile: Lock,
    env: &mut Environment,
    src_path_storage: Option<CreateSrcPathStorage>,
    remote_src_storage: Option<CreateRemoteSrcStorage>,
    kpar_path_storage: Option<CreateKParPathStorage>,
    remote_kpar_storage: Option<CreateRemoteKParStorage>,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
) -> Result<(), SyncError<UrlParseError>>
where
    Environment: ReadEnvironment + WriteEnvironment,
    CreateSrcPathStorage: Fn(String) -> SrcPathStorage,
    SrcPathStorage: ProjectRead,
    CreateRemoteSrcStorage: Fn(String) -> Result<RemoteSrcStorage, UrlParseError>,
    RemoteSrcStorage: ProjectRead,
    CreateKParPathStorage: Fn(String) -> KParPathStorage,
    KParPathStorage: ProjectRead,
    CreateRemoteKParStorage: Fn(String) -> Result<RemoteKParStorage, UrlParseError>,
    RemoteKParStorage: ProjectRead,
{
    let syncing = "Syncing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{syncing:>12}{header:#} env");

    let mut updated = false;
    'main_loop: for project in lockfile.projects {
        // TODO: We need a proper way to treat multiple IRIs here
        let main_uri = project.identifiers.first().cloned();

        for iri in &project.identifiers {
            let excluded_versions = if let Ok(parsed_iri) = fluent_uri::Iri::parse(iri.clone()) {
                provided_iris.get(parsed_iri.normalize().as_str())
            } else {
                provided_iris.get(iri.as_str())
            };

            let checksum = &project.checksum;
            if let Some(versions) = excluded_versions {
                let mut provided = vec![];

                for project_version in versions {
                    if let Some(provided_checksum) =
                        project_version.checksum_canonical_hex().ok().flatten()
                    {
                        if checksum == &provided_checksum {
                            log::debug!("`{}` is marked as provided, skipping installation", iri);
                            continue 'main_loop;
                        }

                        provided.push(provided_checksum);
                    } else {
                        log::debug!(
                            "failed to get checksum for provided project: {:?}",
                            project_version
                        );
                    }
                }

                return Err(SyncError::InvalidProvidedVersion {
                    iri: iri.as_str().into(),
                    hash: project.checksum.as_str().into(),
                    provided,
                });
            }
        }

        if project.sources.is_empty() {
            return Err(SyncError::MissingSource(
                project.identifiers.as_slice().into(),
            ));
        }

        for uri in &project.identifiers {
            if is_installed(uri, &project.checksum, env)? {
                log::debug!("{} found in sysand_env", &uri);
                continue 'main_loop;
            }
        }

        let mut no_supported = true;
        for source in project.sources {
            let mut supported = true;
            match source {
                Source::Editable { .. } => {
                    // Nothing to install for editable
                }
                Source::LocalSrc { src_path } => {
                    let uri = main_uri
                        .as_ref()
                        .ok_or_else(|| SyncError::MissingIriSrcPath(src_path.as_str().into()))?;
                    let src_path_storage = src_path_storage
                        .as_ref()
                        .ok_or_else(|| SyncError::MissingSrcPathStorage(uri.as_str().into()))?;
                    let storage = src_path_storage(src_path.clone());
                    log::debug!("trying to install `{uri}` from src_path `{src_path}`");
                    try_install(uri, &project.checksum, storage, env)?;
                }
                Source::RemoteSrc { remote_src } => {
                    let uri = main_uri.as_ref().ok_or_else(|| {
                        SyncError::MissingIriRemoteSrc(remote_src.as_str().into())
                    })?;
                    let remote_src_storage = remote_src_storage
                        .as_ref()
                        .ok_or_else(|| SyncError::MissingRemoteSrcStorage(uri.as_str().into()))?;
                    let storage = remote_src_storage(remote_src.clone()).map_err(|e| {
                        SyncError::InvalidRemoteSource(remote_src.as_str().into(), e)
                    })?;
                    log::debug!("trying to install `{uri}` from remote_src: {remote_src}");
                    try_install(uri, &project.checksum, storage, env)?;
                }
                Source::LocalKpar { kpar_path } => {
                    let uri = main_uri.as_ref().ok_or_else(|| {
                        SyncError::MissingIriLocalKparPath(kpar_path.as_str().into())
                    })?;
                    let kpar_path_storage = kpar_path_storage.as_ref().ok_or_else(|| {
                        SyncError::MissingLocalKparStorage(kpar_path.as_str().into())
                    })?;
                    let storage = kpar_path_storage(kpar_path.clone());
                    log::debug!("trying to install `{uri}` from kpar_path: {kpar_path}");
                    try_install(uri, &project.checksum, storage, env)?;
                }
                Source::RemoteKpar {
                    remote_kpar,
                    remote_kpar_size: _,
                } => {
                    let uri = main_uri.as_ref().ok_or_else(|| {
                        SyncError::MissingIriRemoteKparPath(remote_kpar.as_str().into())
                    })?;
                    let remote_kpar_storage = remote_kpar_storage.as_ref().ok_or_else(|| {
                        SyncError::MissingRemoteKparStorage(remote_kpar.as_str().into())
                    })?;
                    let storage = remote_kpar_storage(remote_kpar.clone()).map_err(|e| {
                        SyncError::InvalidRemoteSource(remote_kpar.as_str().into(), e)
                    })?;
                    log::debug!("trying to install `{uri}` from remote_kpar: {remote_kpar}");
                    try_install(uri, &project.checksum, storage, env)?;
                }
                _ => supported = false,
            }
            if supported {
                no_supported = false;
            }
        }
        if no_supported {
            return Err(SyncError::UnsupportedSources(
                main_uri.unwrap_or("project without IRI".to_string()),
            ));
        }
        updated = true;
    }
    if !updated {
        log::info!("{:>12} nothing to do: env is already up to date", ' ');
    }
    Ok(())
}

fn is_installed<E: ReadEnvironment, U, Str1: AsRef<str>, Str2: AsRef<str>>(
    uri: Str1,
    checksum: Str2,
    env: &E,
) -> Result<bool, SyncError<U>> {
    if !env
        .has(&uri)
        .map_err(|e| SyncError::ProjectRead(e.to_string()))?
    {
        return Ok(false);
    }
    for version in env
        .versions(&uri)
        .map_err(|e| SyncError::ProjectRead(e.to_string()))?
    {
        let version: String = version.map_err(|e| SyncError::ProjectRead(e.to_string()))?;
        let project_checksum = env
            .get_project(&uri, version)
            .map_err(|e| SyncError::ProjectRead(e.to_string()))?
            .checksum_noncanonical_hex()
            .map_err(|e| SyncError::ProjectRead(e.to_string()))?
            .ok_or_else(|| SyncError::BadProject(uri.as_ref().to_owned()))?;
        if checksum.as_ref() == project_checksum {
            return Ok(true);
        }
    }
    Ok(false)
}

fn try_install<
    E: ReadEnvironment + WriteEnvironment,
    P: ProjectRead,
    U,
    Str1: AsRef<str>,
    Str2: AsRef<str>,
>(
    uri: Str1,
    checksum: Str2,
    storage: P,
    env: &mut E,
) -> Result<(), SyncError<U>> {
    let project_checksum = storage
        .checksum_canonical_hex()
        .map_err(|e| SyncError::ProjectRead(e.to_string()))?
        .ok_or_else(|| SyncError::BadProject(uri.as_ref().to_owned()))?;
    if checksum.as_ref() == project_checksum {
        // TODO: Need to decide how to handle existing installations and possible flags to modify behavior
        do_env_install_project(&uri, &storage, env, true, true).map_err(|e| {
            SyncError::InstallFail {
                uri: uri.as_ref().into(),
                cause: e.to_string(),
            }
        })?;
    } else {
        log::debug!("incorrect checksum for `{}` in lockfile", uri.as_ref());
        log::debug!("lockfile checksum = `{}`", checksum.as_ref());
        log::debug!("project checksum = `{}`", project_checksum);
        return Err(SyncError::BadChecksum(uri.as_ref().into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use indexmap::IndexMap;
    use semver::Version;

    use crate::{
        env::{
            ReadEnvironment, WriteEnvironment, memory::MemoryStorageEnvironment,
            utils::clone_project,
        },
        model::{InterchangeProjectInfo, InterchangeProjectMetadata},
        project::{ProjectMut, ProjectRead, memory::InMemoryProject},
        sync::{SyncError, is_installed, try_install},
    };

    #[derive(Debug)]
    struct E;

    fn storage_example() -> InMemoryProject {
        let mut storage = InMemoryProject::new();

        storage
            .put_project(
                &InterchangeProjectInfo {
                    name: "install_test".to_string(),
                    description: None,
                    version: Version::new(1, 2, 3),
                    license: None,
                    maintainer: vec![],
                    website: None,
                    topic: vec![],
                    usage: vec![],
                }
                .into(),
                &InterchangeProjectMetadata {
                    index: IndexMap::new(),
                    created: DateTime::from_timestamp(1, 2).unwrap(),
                    metamodel: None,
                    includes_derived: None,
                    includes_implied: None,
                    checksum: None,
                }
                .into(),
                true,
            )
            .unwrap();

        storage
    }

    #[test]
    fn test_is_not_installed() {
        let uri = "urn:kpar:install_test";
        let checksum = "00";
        let env = MemoryStorageEnvironment::new();

        assert!(!is_installed::<MemoryStorageEnvironment, u32, _, _>(uri, checksum, &env).unwrap());
    }

    #[test]
    fn test_is_installed() {
        let storage = storage_example();

        let uri = "urn:kpar:install_test";
        let checksum = storage.checksum_noncanonical_hex().unwrap().unwrap();
        let mut env = MemoryStorageEnvironment::new();
        env.put_project(uri, "1,2,3", |p| {
            clone_project(&storage, p, true).map(|_| ())
        })
        .unwrap();

        assert!(is_installed::<MemoryStorageEnvironment, E, _, _>(uri, &checksum, &env).unwrap());

        assert!(!is_installed::<MemoryStorageEnvironment, E, _, _>(uri, "00", &env).unwrap());

        assert!(
            !is_installed::<MemoryStorageEnvironment, E, _, _>("not_uri", &checksum, &env).unwrap()
        );
    }

    #[test]
    fn test_try_install() {
        let storage = storage_example();

        let uri = "urn:kpar:install_test";
        let checksum = storage.checksum_noncanonical_hex().unwrap().unwrap();
        let mut env = MemoryStorageEnvironment::new();

        try_install::<MemoryStorageEnvironment, InMemoryProject, E, _, _>(
            uri, &checksum, storage, &mut env,
        )
        .unwrap();

        let uris = env.uris().unwrap();

        assert_eq!(uris.len(), 1);
        assert_eq!(uris.first().unwrap().as_ref().unwrap(), uri);

        let versions = env.versions(uri).unwrap();

        assert_eq!(versions.len(), 1);
        assert_eq!(versions.first().unwrap().as_ref().unwrap(), "1.2.3");
    }

    #[test]
    fn test_try_install_bad_checksum() {
        let storage = storage_example();

        let uri = "urn:kpar:install_test";
        let checksum = "00";
        let mut env = MemoryStorageEnvironment::new();

        let SyncError::BadChecksum(msg) =
            try_install::<MemoryStorageEnvironment, InMemoryProject, E, _, _>(
                &uri, &checksum, storage, &mut env,
            )
            .unwrap_err()
        else {
            panic!()
        };

        assert_eq!(msg, uri);

        let uris = env.uris().unwrap();

        assert_eq!(uris.len(), 0);
    }
}
