// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

// Resolver for file:// URLs

use std::io::Read;

use crate::{
    project::{ProjectRead, local_kpar::LocalKParProject, local_src::LocalSrcProject},
    resolve::{ResolutionOutcome, ResolveRead},
};

use fluent_uri::component::Scheme;
use thiserror::Error;

/// Resolver for resolving `file://` URIs.
#[derive(Debug)]
pub struct FileResolver {
    /// Relative URIs are resolved with respect to this root.
    pub relative_path_root: Option<std::path::PathBuf>,
    /// This field enables sandboxing the resolved path. If field is not `None`,
    /// the resolved path must be inside at least one of these directories.
    pub sandbox_roots: Option<Vec<std::path::PathBuf>>,
}

#[derive(Error, Debug)]
pub enum FileResolverError {
    #[error("invalid path error")]
    InvalidPath(String),
    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),
}

pub const SCHEME_FILE: &Scheme = Scheme::new_or_panic("file");

fn try_file_uri_to_path(uri: fluent_uri::Iri<String>) -> Option<std::path::PathBuf> {
    if uri.scheme() != SCHEME_FILE {
        return None;
    }

    let url = url::Url::parse(uri.as_str()).ok()?;

    url.to_file_path().ok()
}

impl FileResolver {
    fn resolve_platform_path(
        &self,
        path: std::path::PathBuf,
    ) -> Result<ResolutionOutcome<std::path::PathBuf>, FileResolverError> {
        // Try to resolve relative paths
        let project_path: std::path::PathBuf = if path.is_relative() {
            if let Some(root_part) = &self.relative_path_root {
                let root_part: std::path::PathBuf = root_part.into();
                root_part.join(&path)
            } else {
                return Ok(ResolutionOutcome::UnsupportedIRIType(format!(
                    "Cannot resolve relative file without a specified root directory: {}",
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
                let sandbox_root_canonical = sandbox_root.canonicalize()?;
                let project_path_canonical = project_path.canonicalize()?;

                if project_path_canonical.starts_with(&sandbox_root_canonical) {
                    found = true;
                    break;
                }
                sandbox_roots_canonical.push(sandbox_root_canonical.display().to_string());
            }
            if !found {
                return Ok(ResolutionOutcome::Unresolvable(format!(
                    "Refusing to resolve path {}, is not inside in any of the allowed directories {}",
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
    ) -> Result<ResolutionOutcome<std::path::PathBuf>, FileResolverError> {
        if let Some(file_path) = try_file_uri_to_path(uri.clone()) {
            self.resolve_platform_path(file_path)
        } else {
            Ok(ResolutionOutcome::UnsupportedIRIType(format!(
                "Not a valid file URL: {}",
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

#[derive(Error, Debug)]
pub enum FileResolverProjectError {
    #[error("{0}")]
    ZipError(#[from] zip::result::ZipError),
    #[error("invalid name in archive: {0}")]
    NameError(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    SerdeError(#[from] serde_json::Error),
    #[error("{0}")]
    IOError(#[from] std::io::Error),
    #[error("path error")]
    PathError(#[from] crate::project::local_src::PathError),
    #[error("{0}")]
    OtherError(String),
}

pub enum FileResolverProjectReader<'a> {
    File(<crate::project::local_src::LocalSrcProject as ProjectRead>::SourceReader<'a>),
    Archive(<crate::project::local_kpar::LocalKParProject as ProjectRead>::SourceReader<'a>),
}

impl Read for FileResolverProjectReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            FileResolverProjectReader::File(file) => file.read(buf),
            FileResolverProjectReader::Archive(zip_index_reader) => zip_index_reader.read(buf),
        }
    }
}

impl From<crate::project::local_kpar::LocalKParError> for FileResolverProjectError {
    fn from(value: crate::project::local_kpar::LocalKParError) -> Self {
        match value {
            crate::project::local_kpar::LocalKParError::Zip(zip_error) => {
                FileResolverProjectError::ZipError(zip_error)
            }
            crate::project::local_kpar::LocalKParError::InvalidName(err) => {
                FileResolverProjectError::NameError(err)
            }
            crate::project::local_kpar::LocalKParError::NotFound(err) => {
                FileResolverProjectError::NotFound(err)
            }
            crate::project::local_kpar::LocalKParError::Serde(error) => {
                FileResolverProjectError::SerdeError(error)
            }
            crate::project::local_kpar::LocalKParError::Io(error) => {
                FileResolverProjectError::IOError(error)
            }
        }
    }
}

impl From<crate::project::local_src::LocalSrcError> for FileResolverProjectError {
    fn from(value: crate::project::local_src::LocalSrcError) -> Self {
        match value {
            crate::project::local_src::LocalSrcError::Serde(error) => {
                FileResolverProjectError::SerdeError(error)
            }
            crate::project::local_src::LocalSrcError::Io(error) => {
                FileResolverProjectError::IOError(error)
            }
            crate::project::local_src::LocalSrcError::Path(path_error) => {
                FileResolverProjectError::PathError(path_error)
            }
            crate::project::local_src::LocalSrcError::AlreadyExists(msg) => {
                FileResolverProjectError::OtherError(format!("unexpected internal error: {}", msg))
            }
        }
    }
}

impl ProjectRead for FileResolverProject {
    type Error = FileResolverProjectError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
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
