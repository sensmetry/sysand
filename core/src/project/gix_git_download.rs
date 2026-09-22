// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::num::NonZero;

use camino::Utf8PathBuf;
use fluent_uri::Iri;
use gix::{prepare_clone, remote::fetch::Shallow};
use thiserror::Error;

use crate::{
    cancel::CancellationToken,
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectRead,
        local_src::{LocalSrcError, LocalSrcProject, PathError},
        utils::{FileWithLifetime, RelativizePathError, ToPathBuf as _},
    },
};

use super::utils::{FsIoError, ProjectDeserializationError, ProjectSerializationError, wrapfs};

#[derive(Debug)]
pub struct GixDownloadedProject {
    pub url: gix::Url,
    tmp_dir: camino_tempfile::Utf8TempDir,
    inner: LocalSrcProject,
    /// Observed by the clone through [`crate::cancel::GitInterrupt`].
    token: CancellationToken,
}

#[derive(Error, Debug)]
pub enum GixDownloadedError {
    #[error("git clone from `{0}` failed: {1}")]
    Clone(String, Box<gix::Error>),
    /// The clone was cancelled, before it began or while gix was
    /// fetching or checking out.
    #[error("cancelled")]
    Cancelled,
    #[error("failed to parse git URL `{0}`: {1}")]
    UrlParse(Box<str>, Box<gix::Error>),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Serialize(#[from] ProjectSerializationError),
    #[error("git fetch from `{0}` failed: {1}")]
    Fetch(String, Box<gix::Error>),
    #[error("git checkout in temporary directory `{0}` failed: {1}")]
    Checkout(Utf8PathBuf, Box<gix::Error>),
    #[error(
        "cannot construct a relative path from the workspace/project
        directory to one of its dependencies' directory:\n\
        {0}"
    )]
    ImpossibleRelativePath(#[from] RelativizePathError),
    #[error("project is missing metadata file `.meta.json`")]
    MissingMeta,
    #[error("project is missing `.project.json` and/or `.meta.json` files")]
    MissingInfoMeta,
    #[error("{0}")]
    Other(String),
}

impl From<FsIoError> for GixDownloadedError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl From<LocalSrcError> for GixDownloadedError {
    fn from(value: LocalSrcError) -> Self {
        match value {
            LocalSrcError::Deserialize(error) => Self::Deserialize(error),
            LocalSrcError::Path(error) => Self::Path(error),
            LocalSrcError::AlreadyExists(msg) => {
                Self::Other(format!("unexpected internal error: {msg}"))
            }
            LocalSrcError::Io(e) => Self::Io(e),
            LocalSrcError::Serialize(error) => Self::Serialize(error),
            LocalSrcError::ImpossibleRelativePath(err) => Self::ImpossibleRelativePath(err),
            LocalSrcError::MissingMeta => Self::MissingMeta,
            error @ LocalSrcError::MissingInfoMeta { missing: _ } => Self::Other(error.to_string()),
            error @ (LocalSrcError::AmbiguousFormat { dir: _, json: _ }
            | LocalSrcError::Symbols(_)
            | LocalSrcError::JsonWriteToTomlProject { dir: _, file: _ }
            | LocalSrcError::TomlWriteToJsonProject { dir: _ }) => Self::Other(error.to_string()),
        }
    }
}

impl GixDownloadedProject {
    pub fn new<S: AsRef<str>>(url: S) -> Result<Self, GixDownloadedError> {
        let tmp_dir = camino_tempfile::tempdir().map_err(FsIoError::MkTempDir)?;

        Ok(Self {
            // TODO: gix::url::parse() accepts bare local paths; we should verify before that
            // it's URL or path as we expected instead of relying on gix::url::Parse to error out
            url: gix::url::parse(url.as_ref())
                .map_err(|e| GixDownloadedError::UrlParse(url.as_ref().into(), Box::new(e)))?,
            inner: LocalSrcProject::new_access(wrapfs::canonicalize(tmp_dir.path())?, None),
            tmp_dir,
            token: CancellationToken::new(),
        })
    }

    /// Observe `token` during the clone.
    #[must_use]
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.token = token;
        self
    }

    fn ensure_downloaded(&self) -> Result<(), GixDownloadedError> {
        if !self.tmp_dir.path().join(".git").is_dir() {
            // One clone at a time per process: gix's interrupt flag is
            // process-global, and the guard makes it this clone's.
            let interrupt = crate::cancel::GitInterrupt::begin(&self.token)
                .map_err(|_cancelled| GixDownloadedError::Cancelled)?;
            let cancelled = |error: GixDownloadedError| {
                if interrupt.is_cancelled() {
                    GixDownloadedError::Cancelled
                } else {
                    error
                }
            };
            // `gix-transport` builds a `reqwest` client of its own, on a
            // spawned thread, so this path needs the process-wide rustls
            // provider even though it never touches `create_reqwest_client`.
            // Without it an https clone fails inside that thread, surfacing
            // as an opaque transport error rather than a TLS one.
            crate::resolve::net_utils::install_default_crypto_provider();

            let prepared_clone = prepare_clone(self.url.clone(), self.tmp_dir.path())
                .map_err(|e| GixDownloadedError::Clone(self.url.to_string(), Box::new(e)))?;

            let (mut prepare_checkout, _) = prepared_clone
                .with_shallow(Shallow::DepthAtRemote(NonZero::new(1).unwrap()))
                .fetch_then_checkout(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
                .map_err(|e| {
                    cancelled(GixDownloadedError::Fetch(self.url.to_string(), Box::new(e)))
                })?;
            let (_repo, _) = prepare_checkout
                .main_worktree(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
                .map_err(|e| {
                    cancelled(GixDownloadedError::Checkout(
                        self.tmp_dir.to_path_buf(),
                        Box::new(e),
                    ))
                })?;
        }

        Ok(())
    }
}

impl ProjectRead for GixDownloadedProject {
    type Error = GixDownloadedError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.ensure_downloaded()?;

        Ok(self.inner.get_project()?)
    }

    type SourceReader<'a>
        = FileWithLifetime<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.ensure_downloaded()?;

        Ok(FileWithLifetime::new(self.inner.read_source(path)?))
    }

    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![Source::RemoteGit {
            // Panic here should be impossible, as the URL should have
            // been created from IRI
            remote_git: Iri::parse(self.url.to_string()).unwrap(),
        }])
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // A git checkout is one project at one revision.
        false
    }

    fn checksum_canonical_variant(&self) -> Result<super::ProjectChecksum, Self::Error> {
        self.ensure_downloaded()?;

        Ok(self.inner.checksum_canonical_variant()?)
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        // FIXME: the URL can be `file:`, but it's tricky to
        // get an absolute path from that, as it's unclear how Windows drive prefixes
        // are handled in gix::url::parse()
        None
    }
}

#[cfg(test)]
#[path = "./gix_git_download_tests.rs"]
pub(crate) mod tests;
