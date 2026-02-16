// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{result::Result, sync::Arc};

use camino::Utf8Path;
use reqwest_middleware::ClientWithMiddleware;
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    auth::HTTPAuthentication,
    env::memory::MemoryStorageEnvironment,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        AsSyncProjectTokio, ProjectRead, ProjectReadAsync,
        editable::EditableProject,
        local_kpar::LocalKParProject,
        local_src::LocalSrcProject,
        reference::ProjectReference,
        reqwest_kpar_download::{ReqwestKparDownloadedError, ReqwestKparDownloadedProject},
        reqwest_src::ReqwestSrcProjectAsync,
        utils::FsIoError,
    },
    resolve::memory::{AcceptAll, MemoryResolver},
};

#[derive(Debug, ProjectRead)]
pub enum AnyProject<Policy: HTTPAuthentication> {
    Editable(EditableProject<LocalSrcProject>),
    LocalSrc(LocalSrcProject),
    LocalKpar(LocalKParProject),
    RemoteSrc(AsSyncProjectTokio<ReqwestSrcProjectAsync<Policy>>),
    RemoteKpar(AsSyncProjectTokio<ReqwestKparDownloadedProject<Policy>>),
}

#[derive(Error, Debug)]
pub enum TryFromSourceError {
    #[error("unsupported source\n{0}")]
    UnsupportedSource(String),
    #[error(transparent)]
    LocalKpar(Box<FsIoError>),
    #[error(transparent)]
    RemoteKpar(ReqwestKparDownloadedError),
    #[error(transparent)]
    RemoteSrc(url::ParseError),
}

// TODO: Find a better solution going from source to project.
// Preferably one that can also be used when syncing.
impl<Policy: HTTPAuthentication> AnyProject<Policy> {
    pub fn try_from_source<P: AsRef<Utf8Path>>(
        source: Source,
        project_root: P,
        auth_policy: Arc<Policy>,
        client: ClientWithMiddleware,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Result<Self, TryFromSourceError> {
        match source {
            Source::Editable { editable } => {
                let project = LocalSrcProject {
                    nominal_path: Some(editable.to_string().into()),
                    project_path: project_root.as_ref().join(editable.as_str()),
                };
                Ok(AnyProject::Editable(
                    EditableProject::<LocalSrcProject>::new(editable.as_str().into(), project),
                ))
            }
            Source::LocalKpar { kpar_path } => Ok(AnyProject::LocalKpar(
                LocalKParProject::new_guess_root_nominal(
                    project_root.as_ref().join(kpar_path.as_str()),
                    kpar_path.as_str(),
                )
                .map_err(TryFromSourceError::LocalKpar)?,
            )),
            Source::LocalSrc { src_path } => {
                let nominal_path = src_path.into_string().into();
                let project_path = project_root.as_ref().join(&nominal_path);
                Ok(AnyProject::LocalSrc(LocalSrcProject {
                    nominal_path: Some(nominal_path),
                    project_path,
                }))
            }
            Source::RemoteKpar {
                remote_kpar,
                remote_kpar_size: _,
            } => Ok(AnyProject::RemoteKpar(
                ReqwestKparDownloadedProject::<Policy>::new_guess_root(
                    remote_kpar,
                    client,
                    auth_policy,
                )
                .map_err(TryFromSourceError::RemoteKpar)?
                .to_tokio_sync(runtime),
            )),
            Source::RemoteSrc { remote_src } => Ok(AnyProject::RemoteSrc(
                ReqwestSrcProjectAsync::<Policy> {
                    client,
                    url: reqwest::Url::parse(&remote_src).map_err(TryFromSourceError::RemoteSrc)?,
                    auth_policy,
                }
                .to_tokio_sync(runtime),
            )),
            _ => Err(TryFromSourceError::UnsupportedSource(format!("{source:?}"))),
        }
    }
}

pub type OverrideProject<Policy> = ProjectReference<AnyProject<Policy>>;

pub type OverrideEnvironment<Policy> = MemoryStorageEnvironment<OverrideProject<Policy>>;

pub type OverrideResolver<Policy> = MemoryResolver<AcceptAll, OverrideProject<Policy>>;
