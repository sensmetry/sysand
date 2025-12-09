// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

// Resolver for file:// URLs

use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        self, ProjectRead,
        editable::GetPath,
        local_kpar::{LocalKParError, LocalKParProject},
        local_src::{LocalSrcError, LocalSrcProject},
        utils::{FsIoError, ProjectDeserializationError, wrapfs},
    },
    resolve::{ResolutionOutcome, ResolveRead},
};

use fluent_uri::component::Scheme;
use thiserror::Error;

/// Resolver for resolving `file://` URIs.
#[derive(Debug)]
pub struct FileResolver {
    /// Relative URIs are resolved with respect to this root.
    pub relative_path_root: Option<PathBuf>,
    /// This field enables sandboxing the resolved path. If field is not `None`,
    /// the resolved path must be inside at least one of these directories.
    pub sandbox_roots: Option<Vec<PathBuf>>,
}

#[derive(Error, Debug)]
pub enum FileResolverError {
    #[error("invalid path `{0}`")]
    InvalidPath(String),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl From<FsIoError> for FileResolverError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

pub const SCHEME_FILE: &Scheme = Scheme::new_or_panic("file");

/// Try to obtain a file path from `uri` with `file` scheme. If path
/// is present, it is always absolute according to URI spec
fn try_file_uri_to_path(uri: &fluent_uri::Iri<String>) -> Option<PathBuf> {
    if uri.scheme() == SCHEME_FILE {
        let url = url::Url::parse(uri.as_str()).ok()?;

        url.to_file_path().ok()
    } else {
        None
    }
}

impl FileResolver {
    fn resolve_platform_path(
        &self,
        path: PathBuf,
    ) -> Result<ResolutionOutcome<PathBuf>, FileResolverError> {
        // Try to resolve relative paths
        let project_path: PathBuf = if path.is_relative() {
            if let Some(root_part) = &self.relative_path_root {
                root_part.join(&path)
            } else {
                return Ok(ResolutionOutcome::UnsupportedIRIType(format!(
                    "cannot resolve relative file without a specified root directory: {}",
                    path.display()
                )));
            }
        } else {
            path
        };

        // Use canonicalised paths to check that the tentative project path is within the "jail"
        if let Some(sandboxed_roots) = &self.sandbox_roots {
            let mut found = false;
            let mut sandbox_roots_canonical = Vec::new();
            for sandbox_root in sandboxed_roots {
                let sandbox_root_canonical = wrapfs::canonicalize(sandbox_root)?;
                let project_path_canonical = wrapfs::canonicalize(&project_path)?;

                if project_path_canonical.starts_with(&sandbox_root_canonical) {
                    found = true;
                    break;
                }
                sandbox_roots_canonical.push(sandbox_root_canonical.display().to_string());
            }
            if !found {
                return Ok(ResolutionOutcome::Unresolvable(format!(
                    "refusing to resolve path `{}`, is not inside in any of the allowed directories\n{}",
                    project_path.display(),
                    sandbox_roots_canonical.join("; "),
                )));
            }
        }

        Ok(ResolutionOutcome::Resolved(project_path))
    }

    fn resolve_general(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<PathBuf>, FileResolverError> {
        if let Some(file_path) = try_file_uri_to_path(uri) {
            self.resolve_platform_path(file_path)
        } else {
            Ok(ResolutionOutcome::UnsupportedIRIType(format!(
                "`{}` is not a valid file URL",
                &uri
            )))
        }
    }
}

#[derive(Debug)]
pub enum FileResolverProject {
    LocalSrcProject(LocalSrcProject),
    LocalKParProject(LocalKParProject),
}

impl GetPath for FileResolverProject {
    fn get_path(&self) -> &str {
        match self {
            FileResolverProject::LocalSrcProject(p) => p.get_path(),
            FileResolverProject::LocalKParProject(p) => p.get_path(),
        }
    }
}

#[derive(Error, Debug)]
pub enum FileResolverProjectError {
    #[error(transparent)]
    Zip(project::utils::ZipArchiveError),
    #[error("path `{0}` not found")]
    NotFound(Box<Path>),
    #[error(transparent)]
    Deserialize(ProjectDeserializationError),
    #[error(transparent)]
    LocalSrc(LocalSrcError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    Path(#[from] project::local_src::PathError),
    #[error("{0}")]
    Other(String),
}

impl From<FsIoError> for FileResolverProjectError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

pub enum FileResolverProjectReader<'a> {
    File(<project::local_src::LocalSrcProject as ProjectRead>::SourceReader<'a>),
    Archive(<project::local_kpar::LocalKParProject as ProjectRead>::SourceReader<'a>),
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

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self {
            FileResolverProject::LocalSrcProject(proj) => proj.sources(),
            FileResolverProject::LocalKParProject(proj) => proj.sources(),
        }
    }
}

impl ResolveRead for FileResolver {
    type Error = FileResolverError;

    type ProjectStorage = FileResolverProject;

    type ResolvedStorages = Vec<Result<Self::ProjectStorage, FileResolverError>>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        Ok(match self.resolve_general(uri)? {
            ResolutionOutcome::Resolved(path) => ResolutionOutcome::Resolved(vec![
                Ok(FileResolverProject::LocalSrcProject(LocalSrcProject {
                    project_path: path.clone(),
                })),
                Ok(FileResolverProject::LocalKParProject(
                    LocalKParProject::new_guess_root(path)?,
                )),
            ]),
            ResolutionOutcome::UnsupportedIRIType(msg) => {
                ResolutionOutcome::UnsupportedIRIType(msg)
            }
            ResolutionOutcome::Unresolvable(msg) => ResolutionOutcome::Unresolvable(msg),
        })
    }
}
