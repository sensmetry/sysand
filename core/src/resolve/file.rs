// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

// Resolver for file:// URLs

use crate::paths::{RelativizePathError, join_lexical, relativize_path};
use crate::project::VersionAndUsage;
use std::{
    io::{self, ErrorKind, Read},
    path::PathBuf,
    sync::Arc,
};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

use crate::{
    context::ProjectContext,
    lock::Source,
    manifest::MODEL_TOML_FILE,
    manifest::edit::DocumentMut,
    model::InterchangeProjectUsage,
    project::{
        self, ProjectDefinition, ProjectFormat, ProjectRead,
        local_kpar::{KparInnerPath, LocalKParError, LocalKParProject},
        local_src::{LocalSrcError, LocalSrcProject, detect_format},
        utils::{FsIoError, ProjectDeserializationError, wrapfs},
    },
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead},
    utils::scheme::SCHEME_FILE,
    workspace::model_toml::{DiscoverError, PathRegion, Workspace, discover_table, project_region},
};

/// Resolver for resolving `file://` URIs.
///
/// `Default` is the plain one: no workspace.
#[derive(Debug, Default)]
pub struct FileResolver {
    /// The workspace a resolved target may be a member of. A member is
    /// read with the manifest the workspace resolved. Set by
    /// [`crate::resolve::ResolveRead::within_workspace`].
    pub workspace: Option<Arc<Workspace>>,
}

impl FileResolver {
    /// The same project, read as a member when it sits inside the workspace
    /// this resolver was given.
    fn as_member(&self, project: LocalSrcProject) -> Result<LocalSrcProject, FileResolverError> {
        let format = detect_format(project.path())?;
        if let Some(workspace) = &self.workspace {
            let root = workspace.root();
            match project_region(root, workspace.config(), project.path(), format) {
                // A loaded workspace has one member per `members` entry, in
                // order.
                PathRegion::Member(index, _) => {
                    let manifest = workspace.members()[index].project.clone();
                    return Ok(project.with_member(manifest));
                }
                // A project under the root that is neither a member nor in
                // `exclude` is an error: it would be recorded in the lockfile
                // like an excluded project, so nothing downstream could tell
                // which workspace owned it.
                PathRegion::Unlisted => {
                    return Err(FileResolverError::Workspace(DiscoverError::Unlisted {
                        project: project.path().to_owned(),
                        root: root.to_owned(),
                    }));
                }
                PathRegion::Excluded | PathRegion::Outside => {}
            }
        }
        Self::as_foreign_member(project, format)
    }

    /// The same project, read as a member of the workspace it sits in when
    /// that is not the workspace this resolver was given (or it was given
    /// none).
    fn as_foreign_member(
        project: LocalSrcProject,
        format: ProjectFormat,
    ) -> Result<LocalSrcProject, FileResolverError> {
        let Some((root, table)) =
            discover_table(project.path()).map_err(FileResolverError::Workspace)?
        else {
            return Ok(project);
        };
        match project_region(&root, &table, project.path(), format) {
            PathRegion::Member(..) if format == ProjectFormat::Json => {
                Err(FileResolverError::ForeignMemberNotModelToml {
                    project: project.path().to_owned(),
                    root,
                })
            }
            // Reading the project reports a directory with both manifests
            // or neither.
            PathRegion::Member(..) if format != ProjectFormat::Toml => Ok(project),
            PathRegion::Member(index, _) => {
                let text = wrapfs::read_to_string(project.path().join(MODEL_TOML_FILE))?;
                let file = format!("{}/{MODEL_TOML_FILE}", table.members[index].path);
                let manifest = table
                    .resolve_member_text(index, &file, &text)
                    .map_err(FileResolverError::ForeignMember)?;
                Ok(project.with_member(manifest))
            }
            PathRegion::Excluded | PathRegion::Outside => Ok(project),
            PathRegion::Unlisted => Err(FileResolverError::Workspace(DiscoverError::Unlisted {
                project: project.path().to_owned(),
                root,
            })),
        }
    }

    /// The same rule for the path a `file:` resource names.
    ///
    /// A `file:` resource is two candidates at once (an unpacked directory
    /// and an archive), and only a directory is a project the rule is
    /// about. An archive *file* under the root must keep resolving:
    /// `output/<name>.kpar` is where this workspace's own builds land.
    fn check_resource_listed(&self, path: &Utf8Path) -> Result<(), FileResolverError> {
        let Some(workspace) = &self.workspace else {
            return Ok(());
        };
        let root = workspace.root();
        if !wrapfs::is_dir(path)? {
            return Ok(());
        }
        // `region_of` compares against the canonical root, while a URL's
        // path is as written (e.g. `/var` for `/private/var` on macOS)
        let canonical = wrapfs::canonicalize(path)?;
        let format = detect_format(&canonical)?;
        match project_region(root, workspace.config(), &canonical, format) {
            PathRegion::Unlisted => Err(FileResolverError::Workspace(DiscoverError::Unlisted {
                project: path.to_owned(),
                root: root.to_owned(),
            })),
            PathRegion::Member(..) | PathRegion::Excluded | PathRegion::Outside => Ok(()),
        }
    }
}

#[derive(Error, Debug)]
pub enum FileResolverError {
    /// A target that lies under the workspace root without being a
    /// member. Carries [`DiscoverError`].
    #[error(transparent)]
    Workspace(DiscoverError),
    /// The `model.toml` of a member of another workspace is not valid.
    #[error(transparent)]
    ForeignMember(ProjectDeserializationError),
    /// A directory another workspace lists in `members` is not a
    /// `model.toml` project.
    #[error(
        "`{project}` is a member of the workspace at `{root}` but is not a `model.toml` project"
    )]
    ForeignMemberNotModelToml {
        project: Utf8PathBuf,
        root: Utf8PathBuf,
    },
    #[error("failed to encode path `{0}` in UTF-8")]
    InvalidPath(PathBuf),
    #[error("IRI `{0}` is not a valid URL")]
    IriNotValidUrl(String, #[source] url::ParseError),
    #[error("failed to extract path from file URL")]
    FailedPathExtract,
    #[error("path `{path}` climbs above the filesystem root from `{base}`")]
    PathAboveRoot { base: Utf8PathBuf, path: String },
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

/// Resolve `relative` against `base` and canonicalize it.
///
/// `base` must be canonical. Each `..` of `relative` cancels the
/// component before it as text, a symlink included, and one that climbs
/// above the root is refused.
fn resolve_relative_path(
    base: &Utf8Path,
    relative: &str,
) -> Result<ResolutionOutcome<Utf8PathBuf>, FileResolverError> {
    let Some(abs_path) = join_lexical(base, Utf8Path::new(relative)) else {
        return Err(FileResolverError::PathAboveRoot {
            base: base.to_owned(),
            path: relative.to_owned(),
        });
    };
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
    Ok(ResolutionOutcome::Resolved(canonical))
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
        directory to one of its dependencies' directory"
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

#[derive(Debug)]
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
            error @ LocalKParError::Format {
                path: _,
                problem: _,
            } => Self::Other(error.to_string()),
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
    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        match self {
            Self::LocalSrcProject(project) => Ok(project.format()?),
            Self::LocalKParProject(project) => Ok(project.format()?),
        }
    }

    /// Forwarded for the same reason as [`Self::format`].
    fn manifest_document(&self) -> Result<DocumentMut, project::ManifestError<Self::Error>> {
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

    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
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

    /// A local directory is locked as an editable project, whatever its
    /// manifest; a KPAR by its path and digest.
    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        match self {
            Self::LocalSrcProject(proj) => {
                let editable = if let Some(nominal) = proj.nominal_path() {
                    nominal.as_str().into()
                } else if let Some(root) = ctx.sources_root() {
                    relativize_path(proj.path(), &root)?.into_string().into()
                } else {
                    panic!("a directory without `nominal_path` has no lockfile source");
                };
                Ok(vec![Source::Editable { editable }])
            }
            Self::LocalKParProject(proj) => Ok(proj.sources(ctx)?),
        }
    }

    fn version_and_usage(&self) -> Result<Option<VersionAndUsage>, Self::Error> {
        match self {
            Self::LocalSrcProject(proj) => Ok(proj.version_and_usage()?),
            Self::LocalKParProject(proj) => Ok(proj.version_and_usage()?),
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
    fn within_workspace(mut self, workspace: Arc<Workspace>) -> Self {
        self.workspace = Some(workspace);
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
                    self.check_resource_listed(&path)?;
                    Ok(ResolutionOutcome::Resolved(vec![
                        Ok(FileResolverProject::LocalSrcProject(
                            LocalSrcProject::new_access(path.clone(), None),
                        )),
                        Ok(FileResolverProject::LocalKParProject(
                            LocalKParProject::new_access(path, KparInnerPath::Guess, None),
                        )),
                    ]))
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
                    let res = resolve_relative_path(base, dir.as_str())?;
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
                    let res = resolve_relative_path(base, kpar_path.as_str())?;
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
            InterchangeProjectUsage::RemoteKpar {
                kpar_url: _,
                publisher: _,
                name: _,
            }
            | InterchangeProjectUsage::Index {
                publisher: _,
                name: _,
                version_constraint: _,
            } => Ok(ResolutionOutcome::UnsupportedUsageType {
                reason: String::from("not a path usage"),
            }),
        }
    }
}
