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
        AsSyncProjectTokio, ProjectRead, ProjectReadAsync as _,
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
    resolve::memory::{AcceptResource, MemoryResolver},
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
    /// An `[override]` naming a project under a workspace root that is
    /// in neither `members` nor `exclude`.
    #[error(transparent)]
    Workspace(#[from] crate::workspace::model_toml::DiscoverError),
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
                // A configured `[override]` may name a workspace member,
                // whose manifest resolves only against its table: read it
                // table-less and a member inheriting its `metamodel`
                // fails with "the project is not a member of any
                // workspace", about a project that is listed in
                // `members`.
                let project = crate::workspace::model_toml::project_at_path(
                    project_root.as_ref().join(editable.as_str()),
                    Some(editable.to_string().into()),
                )?;
                Ok(Self::Editable(EditableProject::<LocalSrcProject>::new(
                    editable.as_str().into(),
                    project,
                )))
            }
            OverrideSource::LocalKpar { kpar_path } => {
                Ok(Self::LocalKpar(LocalKParProject::new_access(
                    project_root.as_ref().join(kpar_path.as_str()),
                    KparInnerPath::Guess,
                    Some(kpar_path),
                )))
            }
            OverrideSource::LocalSrc { src_path } => {
                // As for `Editable` above.
                let project_path = project_root.as_ref().join(src_path.as_str());
                Ok(Self::LocalSrc(
                    crate::workspace::model_toml::project_at_path(project_path, Some(src_path))?,
                ))
            }
            // TODO: use expected size
            OverrideSource::RemoteKpar { remote_kpar } => Ok(Self::RemoteKpar(
                ReqwestRemoteKparDownloadedProject::<Policy>::new_guess_root(
                    remote_kpar,
                    client,
                    auth_policy,
                    None,
                )
                .map_err(TryFromSourceError::RemoteKpar)?
                .to_tokio_sync(runtime),
            )),
            OverrideSource::RemoteSrc { remote_src } => Ok(Self::RemoteSrc(
                ReqwestSrcProjectAsync::<Policy> {
                    client,
                    url: reqwest::Url::parse(remote_src.as_str())
                        .map_err(|e| TryFromSourceError::UrlParse(remote_src.to_string(), e))?,
                    auth_policy,
                    expected_checksum: None,
                }
                .to_tokio_sync(runtime),
            )),
            OverrideSource::RemoteGit { remote_git } => Ok(Self::RemoteGit(
                GixDownloadedProject::new(remote_git).map_err(TryFromSourceError::RemoteGit)?,
            )),
        }
    }
}

pub type OverrideProject<Policy> = ProjectReference<AnyProject<Policy>>;

pub type OverrideEnvironment<Policy> = MemoryStorageEnvironment<OverrideProject<Policy>>;

pub type OverrideResolver<Policy> = MemoryResolver<AcceptResource, OverrideProject<Policy>>;
