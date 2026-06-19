// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

// Resolver for file:// URLs

use std::{
    io::{self, Read},
    path::PathBuf,
};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

use crate::{
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsage},
    project::{
        self, ProjectRead,
        local_kpar::{KparInnerPath, LocalKParError, LocalKParProject},
        local_src::{LocalSrcError, LocalSrcProject},
        utils::{FsIoError, ProjectDeserializationError, RelativizePathError, wrapfs},
    },
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead},
    utils::scheme::SCHEME_FILE,
};

/// Resolver for resolving `file://` URIs.
#[derive(Debug)]
pub struct FileResolver {
    /// This field enables sandboxing the resolved path. If field is not `None`,
    /// the resolved path must be inside at least one of these directories.
    pub sandbox_roots: Option<Vec<Utf8PathBuf>>,
}

#[derive(Error, Debug)]
pub enum FileResolverError {
    #[error("failed to encode path `{0}` in UTF-8")]
    InvalidPath(PathBuf),
    #[error("IRI `{0}` is not a valid URL: {1}")]
    IriNotValidUrl(String, url::ParseError),
    #[error("failed to extract path from file URL")]
    FailedPathExtract,
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl From<FsIoError> for FileResolverError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

/// Try to obtain a file path from `uri` with `file` scheme. If path
/// is present, it is always absolute according to URI spec
fn try_file_uri_to_path(
    uri: &fluent_uri::Iri<String>,
) -> Result<Option<Utf8PathBuf>, FileResolverError> {
    if uri.scheme() == SCHEME_FILE {
        let url = match url::Url::parse(uri.as_str()) {
            Ok(u) => u,
            // This fails only in esoteric cases, such as if host is
            // IPvFuture, which is allowed by IRI, but not WHATWG URL.
            Err(e) => return Err(FileResolverError::IriNotValidUrl(uri.to_string(), e)),
        };

        match url.to_file_path() {
            Ok(p) => Some(Utf8PathBuf::from_path_buf(p).map_err(FileResolverError::InvalidPath))
                .transpose(),
            Err(()) => Err(FileResolverError::FailedPathExtract),
        }
    } else {
        Ok(None)
    }
}

impl FileResolver {
    fn check_sandbox(
        &self,
        path: Utf8PathBuf,
    ) -> Result<ResolutionOutcome<Utf8PathBuf>, FileResolverError> {
        // Use canonicalised paths to check that the tentative project path is within the "jail"
        if let Some(sandboxed_roots) = &self.sandbox_roots {
            let mut found = false;
            let mut sandbox_roots_canonical = Vec::new();
            for sandbox_root in sandboxed_roots {
                let sandbox_root_canonical = wrapfs::canonicalize(sandbox_root)?;
                let project_path_canonical = wrapfs::canonicalize(&path)?;

                if project_path_canonical.starts_with(&sandbox_root_canonical) {
                    found = true;
                    break;
                }
                sandbox_roots_canonical.push(sandbox_root_canonical.to_string());
            }
            if !found {
                return Ok(ResolutionOutcome::Unresolvable {
                    reason: format!(
                        "refusing to resolve path `{path}`, is not inside in any of the allowed directories\n{}",
                        sandbox_roots_canonical.join("; "),
                    ),
                });
            }
        }
        Ok(ResolutionOutcome::Resolved(path))
    }
}

#[derive(Debug)]
pub enum FileResolverProject {
    LocalSrcProject(LocalSrcProject),
    LocalKParProject(LocalKParProject),
}

#[derive(Error, Debug)]
pub enum FileResolverProjectError {
    #[error(transparent)]
    Zip(project::utils::ZipArchiveError),
    #[error("path `{0}` not found")]
    NotFound(Box<Utf8Path>),
    #[error(transparent)]
    Deserialize(ProjectDeserializationError),
    #[error(transparent)]
    LocalSrc(LocalSrcError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    Path(#[from] project::local_src::PathError),
    #[error(
        "cannot construct a relative path from the workspace/project
        directory to one of its dependencies' directory:\n\
        {0}"
    )]
    ImpossibleRelativePath(#[from] RelativizePathError),
    #[error("kpar at `{path}` has sha256 `{computed}` but the expected digest was `{expected}`")]
    DigestMismatch {
        path: Box<str>,
        expected: String,
        computed: String,
    },
    #[error("kpar at `{path}` has size {actual} bytes but the expected size was {expected} bytes")]
    SizeMismatch {
        path: Box<str>,
        expected: u64,
        actual: u64,
    },
    #[error("kpar at `{path}` is an empty file")]
    EmptyKpar { path: Box<str> },
    #[error("{0}")]
    Other(String),
}

impl From<FsIoError> for FileResolverProjectError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

pub enum FileResolverProjectReader<'a> {
    File(<LocalSrcProject as ProjectRead>::SourceReader<'a>),
    Archive(<LocalKParProject as ProjectRead>::SourceReader<'a>),
}

impl Read for FileResolverProjectReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            FileResolverProjectReader::File(file) => file.read(buf),
            FileResolverProjectReader::Archive(zip_index_reader) => zip_index_reader.read(buf),
        }
    }
}

impl From<LocalKParError> for FileResolverProjectError {
    fn from(value: LocalKParError) -> Self {
        match value {
            LocalKParError::NotFound(err) => FileResolverProjectError::NotFound(err),
            LocalKParError::Deserialize(error) => FileResolverProjectError::Deserialize(error),
            LocalKParError::Io(error) => FileResolverProjectError::Io(error),
            LocalKParError::Zip(err) => FileResolverProjectError::Zip(err),
            LocalKParError::ImpossibleRelativePath(err) => Self::ImpossibleRelativePath(err),
            LocalKParError::DigestMismatch {
                path,
                expected,
                computed,
            } => Self::DigestMismatch {
                path,
                expected,
                computed,
            },
            LocalKParError::SizeMismatch {
                path,
                expected,
                actual,
            } => Self::SizeMismatch {
                path,
                expected,
                actual,
            },
            LocalKParError::EmptyKpar { path } => Self::EmptyKpar { path },
        }
    }
}

impl From<LocalSrcError> for FileResolverProjectError {
    fn from(value: LocalSrcError) -> Self {
        match value {
            LocalSrcError::Deserialize(error) => FileResolverProjectError::Deserialize(error),
            LocalSrcError::Path(path_error) => FileResolverProjectError::Path(path_error),
            LocalSrcError::AlreadyExists(msg) => {
                FileResolverProjectError::Other(format!("unexpected internal error: {}", msg))
            }
            e => FileResolverProjectError::LocalSrc(e),
        }
    }
}

impl ProjectRead for FileResolverProject {
    type Error = FileResolverProjectError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        match self {
            FileResolverProject::LocalSrcProject(local_src_project) => {
                Ok(local_src_project.get_project()?)
            }
            FileResolverProject::LocalKParProject(local_kpar_project) => {
                Ok(local_kpar_project.get_project()?)
            }
        }
    }

    type SourceReader<'a>
        = FileResolverProjectReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self {
            FileResolverProject::LocalSrcProject(local_src_project) => Ok(
                FileResolverProjectReader::File(local_src_project.read_source(path)?),
            ),
            FileResolverProject::LocalKParProject(local_kpar_project) => Ok(
                FileResolverProjectReader::Archive(local_kpar_project.read_source(path)?),
            ),
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match self {
            FileResolverProject::LocalSrcProject(proj) => proj.is_definitely_invalid(),
            FileResolverProject::LocalKParProject(proj) => proj.is_definitely_invalid(),
        }
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        match self {
            FileResolverProject::LocalSrcProject(proj) => proj
                .sources(ctx)
                .map_err(FileResolverProjectError::LocalSrc),
            FileResolverProject::LocalKParProject(proj) => Ok(proj.sources(ctx)?),
        }
    }

    fn checksum_canonical_variant(&self) -> Result<project::ProjectChecksum, Self::Error> {
        match self {
            FileResolverProject::LocalSrcProject(proj) => proj
                .checksum_canonical_variant()
                .map_err(FileResolverProjectError::LocalSrc),
            FileResolverProject::LocalKParProject(proj) => Ok(proj.checksum_canonical_variant()?),
        }
    }
}

impl ResolveRead for FileResolver {
    type Error = FileResolverError;

    type ProjectStorage = FileResolverProject;

    type ResolvedStorages = Vec<Result<Self::ProjectStorage, FileResolverError>>;

    fn resolve_read(
        &self,
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        match resolve.usage() {
            InterchangeProjectUsage::Resource {
                resource: url,
                // TODO: check that the project version satisfies this
                version_constraint: _,
            } => match try_file_uri_to_path(url)? {
                Some(path) => {
                    let res = self.check_sandbox(path)?;
                    Ok(res.map(|path| {
                        vec![
                            Ok(FileResolverProject::LocalSrcProject(LocalSrcProject {
                                nominal_path: None,
                                project_path: path.clone(),
                                expected_checksum: None,
                            })),
                            Ok(FileResolverProject::LocalKParProject(
                                LocalKParProject::new(path, KparInnerPath::Guess, None, None),
                            )),
                        ]
                    }))
                }
                None => Ok(ResolutionOutcome::UnsupportedUsageType {
                    reason: String::from("resource is not a file URL"),
                }),
            },
            // TODO: we must check somewhere that publisher/name match actual
            InterchangeProjectUsage::Directory {
                publisher: _,
                name: _,
                dir,
            } => {
                // TODO: should absolute paths be supported here? Cargo does.
                // if path.is_absolute() {
                // self.resolve_platform_path(path.into())
                // } else
                if let Some(base) = resolve.base_path() {
                    let abs_path = base.join(dir.as_str());
                    let res = self.check_sandbox(abs_path)?;
                    Ok(res.map(|path| {
                        vec![Ok(FileResolverProject::LocalSrcProject(LocalSrcProject {
                            nominal_path: None,
                            project_path: path.clone(),
                            expected_checksum: None,
                        }))]
                    }))
                } else {
                    // TODO: return Err?
                    Ok(ResolutionOutcome::Unresolvable {
                        reason: String::from(
                            "cannot resolve relative path usage without a base path",
                        ),
                    })
                }
            }
        }
    }
}
