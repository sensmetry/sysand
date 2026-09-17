// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

// Resolver for file:// URLs

use std::{
    io::{self, ErrorKind, Read},
    path::PathBuf,
};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

use crate::{
    context::ProjectContext,
    lock::Source,
    manifest::workspace_table::WorkspaceTable,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsage},
    project::{
        self, ProjectFormat, ProjectRead,
        local_kpar::{KparInnerPath, LocalKParError, LocalKParProject},
        local_src::{LocalSrcError, LocalSrcProject},
        utils::{FsIoError, ProjectDeserializationError, RelativizePathError, wrapfs},
    },
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead},
    utils::scheme::SCHEME_FILE,
    workspace::model_toml::PathRegion,
};

/// Resolver for resolving `file://` URIs.
///
/// `Default` is the plain one: no sandbox, no workspace.
#[derive(Debug, Default)]
pub struct FileResolver {
    /// This field enables sandboxing the resolved path. If field is not `None`,
    /// the resolved path must be inside at least one of these directories.
    pub sandbox_roots: Option<Vec<Utf8PathBuf>>,
    /// The workspace a resolved target may be a member of: its root and
    /// the table its members are read against. A target under the root
    /// is read as a member, so one that inherits a key can be read at
    /// all. Set by [`crate::resolve::ResolveRead::within_workspace`];
    /// `None` otherwise.
    pub workspace: Option<(Utf8PathBuf, std::sync::Arc<WorkspaceTable>)>,
}

impl FileResolver {
    /// The same project, read as a member when it sits inside the
    /// workspace this resolver was given. A target outside it, or a
    /// resolver with no workspace, is unchanged.
    fn as_member(&self, project: LocalSrcProject) -> Result<LocalSrcProject, FileResolverError> {
        let Some((root, table)) = &self.workspace else {
            return Ok(project);
        };
        match crate::workspace::model_toml::region_of(root, table, project.path()) {
            PathRegion::Member => Ok(project.in_workspace(table.clone())),
            PathRegion::Standalone => Ok(project),
            // `workspaces.md`, Intra-workspace usages: "A `dir` usage
            // from a member to a project that lies under the root
            // without being a member (and not covered in the `exclude`
            // list) is the unlisted-project error of Included
            // projects." Resolved, it would be recorded in the
            // lockfile exactly as an excluded project is -- so nothing
            // downstream could tell which workspace owned it, which is
            // the decidability the rule exists to give.
            PathRegion::Unlisted => Err(FileResolverError::Workspace(
                crate::workspace::model_toml::DiscoverError::Unlisted {
                    project: project.path().to_owned(),
                    root: root.clone(),
                },
            )),
        }
    }

    /// The same rule for a path a `kpar-url` names.
    ///
    /// A `file:` resource is two candidates at once -- an unpacked
    /// directory and an archive -- and only the directory one is a
    /// project under the root that the rule is about. An archive *file*
    /// under the root is not a project directory and must keep
    /// resolving: `output/<name>.kpar` is where this workspace's own
    /// builds land.
    fn resource_is_unlisted(&self, path: &Utf8Path) -> Result<bool, FileResolverError> {
        let Some((root, table)) = &self.workspace else {
            return Ok(false);
        };
        if !crate::project::utils::wrapfs::is_dir(path)? {
            return Ok(false);
        }
        // `region_of` compares against the canonical root, and a URL's
        // path is as written: through a symlink, or `/var` for
        // `/private/var` on macOS, it would read as outside the root
        let path = wrapfs::canonicalize(path)?;
        Ok(matches!(
            crate::workspace::model_toml::region_of(root, table, &path),
            PathRegion::Unlisted
        ))
    }
}

#[derive(Error, Debug)]
pub enum FileResolverError {
    /// A target that lies under the workspace root without being a
    /// member. Carries [`crate::workspace::model_toml::DiscoverError`]
    /// rather than restating its refusal, so that the upward walk and a
    /// usage give one sentence.
    #[error(transparent)]
    Workspace(crate::workspace::model_toml::DiscoverError),
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

    /// Resolve `relative` against `base`, canonicalize it, and check it
    /// against the sandbox roots.
    fn resolve_relative_path(
        &self,
        base: &Utf8Path,
        relative: &str,
    ) -> Result<ResolutionOutcome<Utf8PathBuf>, FileResolverError> {
        let abs_path = base.join(relative);
        let canonical = match wrapfs::canonicalize_raw(&abs_path) {
            Ok(p) => p,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Ok(ResolutionOutcome::NotFound {
                    reason: format!("path `{abs_path}` does not exist"),
                });
            }
            Err(e) => {
                return Err(FileResolverError::Io(
                    FsIoError::Canonicalize(abs_path, e).into(),
                ));
            }
        };
        self.check_sandbox(canonical)
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
            LocalKParError::NotFound(err) => Self::NotFound(err),
            error @ LocalKParError::TomlInKpar { archive: _ } => Self::Other(error.to_string()),
            LocalKParError::Deserialize(error) => Self::Deserialize(error),
            LocalKParError::Io(error) => Self::Io(error),
            LocalKParError::Zip(err) => Self::Zip(err),
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
            LocalSrcError::Deserialize(error) => Self::Deserialize(error),
            LocalSrcError::Path(path_error) => Self::Path(path_error),
            LocalSrcError::AlreadyExists(msg) => {
                Self::Other(format!("unexpected internal error: {msg}"))
            }
            e => Self::LocalSrc(e),
        }
    }
}

impl ProjectRead for FileResolverProject {
    /// Forwarded, not defaulted: a `dir` usage resolves through here, so
    /// the default would claim json for every `model.toml` target.
    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        match self {
            Self::LocalSrcProject(project) => Ok(project.format()?),
            Self::LocalKParProject(project) => Ok(project.format()?),
        }
    }

    /// Forwarded for the same reason as [`Self::format`].
    fn manifest(
        &self,
    ) -> Result<sysand_core::manifest::project::ResolvedProject, project::ManifestError<Self::Error>>
    {
        match self {
            Self::LocalSrcProject(project) => project
                .manifest()
                .map_err(|e| e.map_project_read(Into::into)),
            Self::LocalKParProject(project) => project
                .manifest()
                .map_err(|e| e.map_project_read(Into::into)),
        }
    }

    /// Forwarded for the same reason as [`Self::format`].
    fn manifest_document(
        &self,
    ) -> Result<sysand_core::manifest::edit::DocumentMut, project::ManifestError<Self::Error>> {
        match self {
            Self::LocalSrcProject(project) => project
                .manifest_document()
                .map_err(|e| e.map_project_read(Into::into)),
            Self::LocalKParProject(project) => project
                .manifest_document()
                .map_err(|e| e.map_project_read(Into::into)),
        }
    }

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
            Self::LocalSrcProject(local_src_project) => Ok(local_src_project.get_project()?),
            Self::LocalKParProject(local_kpar_project) => Ok(local_kpar_project.get_project()?),
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
            Self::LocalSrcProject(local_src_project) => Ok(FileResolverProjectReader::File(
                local_src_project.read_source(path)?,
            )),
            Self::LocalKParProject(local_kpar_project) => Ok(FileResolverProjectReader::Archive(
                local_kpar_project.read_source(path)?,
            )),
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match self {
            Self::LocalSrcProject(proj) => proj.is_definitely_invalid(),
            Self::LocalKParProject(proj) => proj.is_definitely_invalid(),
        }
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        match self {
            Self::LocalSrcProject(proj) => proj
                .sources(ctx)
                .map_err(FileResolverProjectError::LocalSrc),
            Self::LocalKParProject(proj) => Ok(proj.sources(ctx)?),
        }
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        match self {
            Self::LocalSrcProject(proj) => proj.source_may_offer_multiple_versions(),
            Self::LocalKParProject(proj) => proj.source_may_offer_multiple_versions(),
        }
    }

    fn checksum_canonical_variant(&self) -> Result<project::ProjectChecksum, Self::Error> {
        match self {
            Self::LocalSrcProject(proj) => proj
                .checksum_canonical_variant()
                .map_err(FileResolverProjectError::LocalSrc),
            Self::LocalKParProject(proj) => Ok(proj.checksum_canonical_variant()?),
        }
    }

    fn project_root(&self) -> Option<&Utf8Path> {
        match self {
            Self::LocalSrcProject(proj) => proj.project_root(),
            Self::LocalKParProject(proj) => proj.project_root(),
        }
    }
}

impl ResolveRead for FileResolver {
    fn within_workspace(
        mut self,
        root: Utf8PathBuf,
        table: std::sync::Arc<WorkspaceTable>,
    ) -> Self {
        self.workspace = Some((root, table));
        self
    }

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
                    // Before the outcome is built, for the reason the
                    // `Directory` arm classifies early: an unlisted
                    // target is an error, not a resolution. Without it a
                    // `kpar-url` was the one spelling of a usage that
                    // walked around the rule, and the unlisted project
                    // was written into the *workspace* lockfile.
                    res.try_map(|path| {
                        if self.resource_is_unlisted(&path)? {
                            let (root, _) = self.workspace.as_ref().expect("checked above");
                            return Err(FileResolverError::Workspace(
                                crate::workspace::model_toml::DiscoverError::Unlisted {
                                    project: path,
                                    root: root.clone(),
                                },
                            ));
                        }
                        Ok(vec![
                            Ok(FileResolverProject::LocalSrcProject(
                                LocalSrcProject::new_access(path.clone(), None),
                            )),
                            Ok(FileResolverProject::LocalKParProject(
                                LocalKParProject::new_access(path, KparInnerPath::Guess, None),
                            )),
                        ])
                    })
                }
                None => Ok(ResolutionOutcome::UnsupportedUsageType {
                    reason: String::from("resource is not a file URL"),
                }),
            },
            InterchangeProjectUsage::Directory {
                dir,
                publisher: _,
                name: _,
            } => {
                // TODO: should absolute paths be supported here? Cargo does.
                if let Some(base) = resolve.base_path() {
                    let res = self.resolve_relative_path(base, dir.as_str())?;
                    // Classified before the outcome is built, because an
                    // unlisted target is an error rather than a resolution.
                    res.try_map(|path| {
                        // The lock checks that the project is
                        // `publisher`/`name`, as for every typed usage
                        let project = self.as_member(LocalSrcProject::new_access(
                            path,
                            // Can't use `dir` here, since `FileResolver` is used for env projects,
                            // and in that case using `dir` would resolve relative to the project
                            // inside `.sysand`, and the dependency would not be found
                            None,
                        ))?;
                        Ok(vec![Ok(FileResolverProject::LocalSrcProject(project))])
                    })
                } else {
                    // TODO: return Err?
                    Ok(ResolutionOutcome::Unresolvable {
                        reason: String::from(
                            "cannot resolve relative path usage without a base path",
                        ),
                    })
                }
            }
            InterchangeProjectUsage::KparPath {
                kpar_path,
                publisher: _,
                name: _,
            } => {
                if let Some(base) = resolve.base_path() {
                    let res = self.resolve_relative_path(base, kpar_path.as_str())?;
                    Ok(res.map(|path| {
                        vec![Ok(FileResolverProject::LocalKParProject(
                            // The lock checks that the project is
                            // `publisher`/`name`, as for every typed usage
                            LocalKParProject::new_access(path, KparInnerPath::Root, None),
                        ))]
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
            InterchangeProjectUsage::Index {
                publisher: _,
                name: _,
                version_constraint: _,
            } => Ok(ResolutionOutcome::UnsupportedUsageType {
                reason: String::from("not a path usage"),
            }),
        }
    }
}
