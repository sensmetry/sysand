// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{result::Result, sync::Arc};

use camino::Utf8Path;
use reqwest_middleware::ClientWithMiddleware;
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    auth::HTTPAuthentication,
    config::OverrideSource,
    context::ProjectContext,
    env::memory::MemoryStorageEnvironment,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        AsSyncProjectTokio, ProjectRead, ProjectReadAsync,
        editable::EditableProject,
        gix_git_download::{GixDownloadedError, GixDownloadedProject},
        local_kpar::LocalKParProject,
        local_src::LocalSrcProject,
        reference::ProjectReference,
        reqwest_kpar_download::{
            ReqwestIndexKparDownloadedProject, ReqwestKparDownloadedError,
            ReqwestRemoteKparDownloadedProject,
        },
        reqwest_src::ReqwestSrcProjectAsync,
        utils::FsIoError,
    },
    resolve::memory::{AcceptAll, MemoryResolver},
};

use super::local_kpar::KparInnerPath;

#[derive(Debug, ProjectRead)]
pub enum AnyProject<Policy: HTTPAuthentication> {
    Editable(EditableProject<LocalSrcProject>),
    LocalSrc(LocalSrcProject),
    LocalKpar(LocalKParProject),
    RemoteSrc(AsSyncProjectTokio<ReqwestSrcProjectAsync<Policy>>),
    RemoteKpar(AsSyncProjectTokio<ReqwestRemoteKparDownloadedProject<Policy>>),
    IndexKpar(AsSyncProjectTokio<ReqwestIndexKparDownloadedProject<Policy>>),
    RemoteGit(GixDownloadedProject),
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
    IndexKpar(ReqwestKparDownloadedError),
    #[error("failed to parse project url `{0}`")]
    UrlParse(String, #[source] url::ParseError),
    #[error(transparent)]
    RemoteGit(GixDownloadedError),
}

// TODO: Find a better solution going from source to project.
impl<Policy: HTTPAuthentication> AnyProject<Policy> {
    pub fn try_from_override_source<P: AsRef<Utf8Path>>(
        source: OverrideSource,
        project_root: P,
        auth_policy: Arc<Policy>,
        client: ClientWithMiddleware,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Result<Self, TryFromSourceError> {
        match source {
            OverrideSource::Editable { editable } => {
                let project = LocalSrcProject {
                    nominal_path: Some(editable.to_string().into()),
                    project_path: project_root.as_ref().join(editable.as_str()),
                    expected_checksum: None,
                };
                Ok(AnyProject::Editable(
                    EditableProject::<LocalSrcProject>::new(editable.as_str().into(), project),
                ))
            }
            OverrideSource::LocalKpar { kpar_path } => {
                Ok(AnyProject::LocalKpar(LocalKParProject::new(
                    project_root.as_ref().join(kpar_path.as_str()),
                    KparInnerPath::Guess,
                    Some(kpar_path),
                    None,
                )))
            }
            OverrideSource::LocalSrc { src_path } => {
                let project_path = project_root.as_ref().join(src_path.as_str());
                Ok(AnyProject::LocalSrc(LocalSrcProject {
                    nominal_path: Some(src_path),
                    project_path,
                    expected_checksum: None,
                }))
            }
            // TODO: use expected size
            OverrideSource::RemoteKpar { remote_kpar } => Ok(AnyProject::RemoteKpar(
                ReqwestRemoteKparDownloadedProject::<Policy>::new_guess_root(
                    remote_kpar,
                    client,
                    auth_policy,
                    None,
                )
                .map_err(TryFromSourceError::RemoteKpar)?
                .to_tokio_sync(runtime),
            )),
            OverrideSource::RemoteSrc { remote_src } => Ok(AnyProject::RemoteSrc(
                ReqwestSrcProjectAsync::<Policy> {
                    client,
                    url: reqwest::Url::parse(&remote_src)
                        .map_err(|e| TryFromSourceError::UrlParse(remote_src, e))?,
                    auth_policy,
                    expected_checksum: None,
                }
                .to_tokio_sync(runtime),
            )),
            OverrideSource::RemoteGit { remote_git } => Ok(AnyProject::RemoteGit(
                GixDownloadedProject::new(remote_git).map_err(TryFromSourceError::RemoteGit)?,
            )),
        }
    }
}

pub type OverrideProject<Policy> = ProjectReference<AnyProject<Policy>>;

pub type OverrideEnvironment<Policy> = MemoryStorageEnvironment<OverrideProject<Policy>>;

pub type OverrideResolver<Policy> = MemoryResolver<AcceptAll, OverrideProject<Policy>>;
