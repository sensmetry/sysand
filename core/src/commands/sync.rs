// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use crate::{
    commands::env::do_env_install_project,
    env::{ReadEnvironment, WriteEnvironment},
    lock::{Lock, Source},
    project::ProjectRead,
};

#[derive(Error, Debug)]
pub enum SyncError<UrlError> {
    #[error("incorrect checksum for {0} in lockfile")]
    BadChecksum(String),
    #[error("project {0} missing .project.json or .meta.json")]
    BadProject(String),
    #[error("no source given for {0} in lockfile")]
    MissingSource(String),
    #[error("no IRI given for {0} in lockfile")]
    MissingIri(String),
    #[error("cannot handle source with src_path")]
    MissingSrcPathStorage,
    #[error("cannot handle source with remote_src")]
    MissingRemoteSrcStorage,
    #[error("{0}")]
    InvalidRemoteSource(UrlError),
    #[error("no supported sources for {0}")]
    UnsupportedSources(String),
    #[error("failed to install project {uri}:\n{cause}")]
    InstallFailure { uri: String, cause: String },
    // TODO: less opaque read errors
    #[error("read error")]
    ReadError,
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
    UrlError,
>(
    lockfile: Lock,
    env: &mut Environment,
    src_path_storage: Option<CreateSrcPathStorage>,
    remote_src_storage: Option<CreateRemoteSrcStorage>,
    kpar_path_storage: Option<CreateKParPathStorage>,
    remote_kpar_storage: Option<CreateRemoteKParStorage>,
    exclude_iris: &std::collections::HashSet<String>,
) -> Result<(), SyncError<UrlError>>
where
    Environment: ReadEnvironment + WriteEnvironment,
    CreateSrcPathStorage: Fn(String) -> SrcPathStorage,
    SrcPathStorage: ProjectRead,
    CreateRemoteSrcStorage: Fn(String) -> Result<RemoteSrcStorage, UrlError>,
    RemoteSrcStorage: ProjectRead,
    CreateKParPathStorage: Fn(String) -> KParPathStorage,
    KParPathStorage: ProjectRead,
    CreateRemoteKParStorage: Fn(String) -> Result<RemoteKParStorage, UrlError>,
    RemoteKParStorage: ProjectRead,
{
    let syncing = "Syncing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{syncing:>12}{header:#} env");

    let mut updated = false;
    'main_loop: for project in lockfile.project {
        // TODO: We need a proper way to treat multiple IRIs here
        let main_uri = project.iris.first().cloned();

        if project.sources.is_empty() {
            return Err(SyncError::MissingSource(format!(
                "Project with IRI(s) {:?} has no known sources",
                project.iris
            )));
        }

        for uri in &project.iris {
            if is_installed(uri, &project.checksum, env)? {
                log::debug!("{} found in sysand_env", &uri);
                continue 'main_loop;
            }
        }

        for iri in &project.iris {
            let excluded = if let Ok(parsed_iri) = fluent_uri::Iri::parse(iri.clone()) {
                exclude_iris.contains(parsed_iri.normalize().as_str())
            } else {
                exclude_iris.contains(iri.as_str())
            };

            if excluded {
                log::debug!("{} excluded from installation", iri);
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
                        .ok_or(SyncError::MissingIri(format!("src_path = {src_path}")))?;
                    let src_path_storage = src_path_storage
                        .as_ref()
                        .ok_or(SyncError::MissingSrcPathStorage)?;
                    let storage = src_path_storage(src_path.clone());
                    log::debug!("trying to install {uri} from src_path: {src_path}");
                    try_install(uri, &project.checksum, storage, env)?;
                }
                Source::RemoteSrc { remote_src } => {
                    let uri = main_uri
                        .as_ref()
                        .ok_or(SyncError::MissingIri(format!("remote_src = {remote_src}")))?;
                    let remote_src_storage = remote_src_storage
                        .as_ref()
                        .ok_or(SyncError::MissingRemoteSrcStorage)?;
                    let storage = remote_src_storage(remote_src.clone())
                        .map_err(|e| SyncError::InvalidRemoteSource(e))?;
                    log::debug!("trying to install {uri} from remote_src: {remote_src}");
                    try_install(uri, &project.checksum, storage, env)?;
                }
                Source::LocalKpar { kpar_path } => {
                    let uri = main_uri
                        .as_ref()
                        .ok_or(SyncError::MissingIri(format!("kpar_path = {kpar_path}")))?;
                    let kpar_path_storage = kpar_path_storage
                        .as_ref()
                        .ok_or(SyncError::MissingSrcPathStorage)?;
                    let storage = kpar_path_storage(kpar_path.clone());
                    log::debug!("trying to install {uri} from kpar_path: {kpar_path}");
                    try_install(uri, &project.checksum, storage, env)?;
                }
                Source::RemoteKpar {
                    remote_kpar,
                    remote_kpar_size: _,
                } => {
                    let uri = main_uri.as_ref().ok_or(SyncError::MissingIri(format!(
                        "remote_kpar = {remote_kpar}"
                    )))?;
                    let remote_kpar_storage = remote_kpar_storage
                        .as_ref()
                        .ok_or(SyncError::MissingRemoteSrcStorage)?;
                    let storage = remote_kpar_storage(remote_kpar.clone())
                        .map_err(|e| SyncError::InvalidRemoteSource(e))?;
                    log::debug!("trying to install {uri} from remote_kpar: {remote_kpar}");
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
        log::info!("env is up to date");
    }
    Ok(())
}

fn is_installed<E: ReadEnvironment, U>(
    uri: &String,
    checksum: &String,
    env: &E,
) -> Result<bool, SyncError<U>> {
    if !env.has(uri).map_err(|_| SyncError::ReadError)? {
        return Ok(false);
    }
    for version in env.versions(uri).map_err(|_| SyncError::ReadError)? {
        let version: String = version.map_err(|_| SyncError::ReadError)?;
        let project_checksum = env
            .get_project(uri, version)
            .map_err(|_| SyncError::ReadError)?
            .checksum_noncanonical_hex()
            .map_err(|_| SyncError::ReadError)?
            .ok_or(SyncError::BadProject(uri.clone()))?;
        if checksum == &project_checksum {
            return Ok(true);
        }
    }
    Ok(false)
}

fn try_install<E: ReadEnvironment + WriteEnvironment, P: ProjectRead, U>(
    uri: &String,
    checksum: &String,
    storage: P,
    env: &mut E,
) -> Result<(), SyncError<U>> {
    let project_checksum = storage
        .checksum_canonical_hex()
        .map_err(|_| SyncError::ReadError)?
        .ok_or(SyncError::BadProject(uri.clone()))?;
    if checksum == &project_checksum {
        // TODO: Need to decide how to handle existing installations and possible flags to modify behavior
        do_env_install_project(uri, storage, env, true, true).map_err(|e| {
            SyncError::InstallFailure {
                uri: uri.to_string(),
                cause: e.to_string(),
            }
        })?;
    } else {
        log::debug!("Incorrect checksum for {} in lockfile", &uri);
        log::debug!("Lockfile checksum = {}", checksum);
        log::debug!("Project checksum = {}", project_checksum);
        return Err(SyncError::BadChecksum(uri.clone()));
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
        let uri = "urn:kpar:install_test".to_string();
        let checksum = "00".to_string();
        let env = MemoryStorageEnvironment::new();

        assert!(!is_installed::<MemoryStorageEnvironment, u32>(&uri, &checksum, &env).unwrap());
    }

    #[test]
    fn test_is_installed() {
        let storage = storage_example();

        let uri = "urn:kpar:install_test".to_string();
        let checksum = storage.checksum_noncanonical_hex().unwrap().unwrap();
        let mut env = MemoryStorageEnvironment::new();
        env.put_project(&uri, "1,2,3", |p| clone_project(&storage, p, true))
            .unwrap();

        assert!(is_installed::<MemoryStorageEnvironment, E>(&uri, &checksum, &env).unwrap());

        assert!(
            !is_installed::<MemoryStorageEnvironment, E>(&uri, &"00".to_string(), &env).unwrap()
        );

        assert!(
            !is_installed::<MemoryStorageEnvironment, E>(&"not_uri".to_string(), &checksum, &env)
                .unwrap()
        );
    }

    #[test]
    fn test_try_install() {
        let storage = storage_example();

        let uri = "urn:kpar:install_test".to_string();
        let checksum = storage.checksum_noncanonical_hex().unwrap().unwrap();
        let mut env = MemoryStorageEnvironment::new();

        try_install::<MemoryStorageEnvironment, InMemoryProject, E>(
            &uri, &checksum, storage, &mut env,
        )
        .unwrap();

        let uris = env.uris().unwrap();

        assert_eq!(uris.len(), 1);
        assert_eq!(uris.first().unwrap().as_ref().unwrap(), &uri);

        let versions = env.versions(uri).unwrap();

        assert_eq!(versions.len(), 1);
        assert_eq!(
            versions.first().unwrap().as_ref().unwrap(),
            &"1.2.3".to_string()
        );
    }

    #[test]
    fn test_try_install_bad_checksum() {
        let storage = storage_example();

        let uri = "urn:kpar:install_test".to_string();
        let checksum = "00".to_string();
        let mut env = MemoryStorageEnvironment::new();

        let SyncError::BadChecksum(msg) =
            try_install::<MemoryStorageEnvironment, InMemoryProject, E>(
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
