// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::paths::{
    NormalizeError, RelativizePathError, ResolvePathError, normalize_relative, relativize_path_in,
};
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Write as _},
};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use indexmap::IndexMap;

use crate::{
    context::ProjectContext,
    env::utils::{CloneError, clone_project},
    lock::Source,
    manifest::{MODEL_TOML_FILE, ParsedModelToml, kpar_view, project::ResolvedProject},
    model::{
        InterchangeProjectInfoPartial, InterchangeProjectInfoRaw,
        InterchangeProjectMetadataPartial, InterchangeProjectMetadataRaw, META_JSON_FILE,
        PROJECT_JSON_FILE,
    },
    project::{
        ManifestError, ProjectDefinition, ProjectMut, ProjectRead,
        utils::{ToPathBuf as _, wrapfs},
    },
};

use super::{
    CanonicalizationError, ProjectChecksum,
    utils::{FsIoError, ProjectDeserializationError, ProjectSerializationError, validate_manifest},
};

pub use crate::project::ProjectFormat;

/// Which manifests `dir` holds.
///
/// Uses `wrapfs::is_file`, so an IO error is reported and a *directory*
/// named `.project.json` is ignored.
pub fn detect_format(dir: &Utf8Path) -> Result<ProjectFormat, Box<FsIoError>> {
    let toml = wrapfs::is_file(dir.join(MODEL_TOML_FILE))?;
    Ok(ProjectFormat::from_presence(
        toml,
        json_manifest(dir)?.is_some(),
    ))
}

/// The json manifest `dir` holds, `.project.json` first when it holds both.
pub(crate) fn json_manifest(dir: &Utf8Path) -> Result<Option<&'static str>, Box<FsIoError>> {
    for file in [PROJECT_JSON_FILE, META_JSON_FILE] {
        if wrapfs::is_file(dir.join(file))? {
            return Ok(Some(file));
        }
    }
    Ok(None)
}

/// Project stored in a local directory as an extracted KPAR archive.
/// Source file paths with (unix) segments `segment1/.../segmentN` are
/// re-interpreted as filesystem-native paths resolved relative to
/// and under `project_path`.
#[derive(Clone, Debug)]
pub struct LocalSrcProject {
    /// Path used in `Source::LocalSrc` returned by `.sources()`.
    /// If `None` no source will be given.
    /// E.g. if used in lockfile would be the path relative to the lockfile.
    // TODO: Consider removing this and replacing it with some way of
    // relativizing `project_path` at the call site of .sources().
    nominal_path: Option<Utf8UnixPathBuf>,
    /// Path used when locating the project internally.
    /// Should be absolute.
    project_path: Utf8PathBuf,
    // TODO: enforce that the project matches the checksum if provided
    // before reading; see LocalKparProject for example
    expected_checksum: Option<String>,
    /// The workspace this project is a member of, if the caller said so.
    ///
    /// A member's manifest may carry inheritance markers, and resolving
    /// with `None` refuses every one of them. The table is never looked up
    /// from the filesystem.
    workspace: Option<std::sync::Arc<crate::manifest::workspace_table::WorkspaceTable>>,
}

impl LocalSrcProject {
    /// Access the project. No invariants will be checked. If `nominal_path` is provided,
    /// `sources()` will work.
    pub fn new_access(path: impl Into<Utf8PathBuf>, nominal_path: Option<Utf8UnixPathBuf>) -> Self {
        Self {
            nominal_path,
            project_path: path.into(),
            expected_checksum: None,
            workspace: None,
        }
    }

    /// Like [`Self::new_access`], for a caller that already knows the
    /// canonical project checksum (e.g. an environment that recorded it at
    /// install time), sparing a hash of the whole project tree.
    pub fn new_access_with_checksum(
        path: impl Into<Utf8PathBuf>,
        nominal_path: Option<Utf8UnixPathBuf>,
        checksum: String,
    ) -> Self {
        Self {
            expected_checksum: Some(checksum),
            ..Self::new_access(path, nominal_path)
        }
    }

    /// The directory this project is read from.
    pub fn path(&self) -> &camino::Utf8Path {
        &self.project_path
    }

    /// The same project, resolving its manifest against `table`. For a
    /// resolver that has found its target to be a member of the workspace
    /// it was given.
    #[must_use]
    pub fn in_workspace(
        mut self,
        table: std::sync::Arc<crate::manifest::workspace_table::WorkspaceTable>,
    ) -> Self {
        self.workspace = Some(table);
        self
    }

    /// [`LocalSrcProject::new_access`] for a workspace member, whose
    /// manifest resolves against `table`.
    pub fn new_member(
        path: impl Into<Utf8PathBuf>,
        nominal_path: Option<Utf8UnixPathBuf>,
        table: std::sync::Arc<crate::manifest::workspace_table::WorkspaceTable>,
    ) -> Self {
        Self {
            workspace: Some(table),
            ..Self::new_access(path, nominal_path)
        }
    }

    pub fn root_path(&self) -> &Utf8Path {
        &self.project_path
    }

    /// The workspace table the manifest resolves against, if the project
    /// was given one.
    pub fn workspace_table(
        &self,
    ) -> Option<&std::sync::Arc<crate::manifest::workspace_table::WorkspaceTable>> {
        self.workspace.as_ref()
    }

    pub fn set_path(&mut self, path: impl Into<Utf8PathBuf>) {
        self.project_path = path.into();
    }

    pub fn nominal_path(&self) -> Option<&Utf8UnixPath> {
        self.nominal_path.as_deref()
    }

    pub fn set_nominal_path(&mut self, path: Option<Utf8UnixPathBuf>) {
        self.nominal_path = path;
    }

    pub fn info_path(&self) -> Utf8PathBuf {
        self.project_path.join(".project.json")
    }

    pub fn meta_path(&self) -> Utf8PathBuf {
        self.project_path.join(".meta.json")
    }

    pub fn get_unix_path<P: AsRef<Utf8Path>>(
        &self,
        path: P,
    ) -> Result<Utf8UnixPathBuf, UnixPathError> {
        let path = relativize_path_in(&path, self.root_path())?
            .ok_or_else(|| UnixPathError::PathOutsideProject(path.to_path_buf()))?;

        Ok(path)
    }

    pub fn get_source_path<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Utf8PathBuf, PathError> {
        let path = path.as_ref();
        let utf_path = if path.is_absolute() {
            if !cfg!(feature = "lenient-checks") {
                return Err(PathError::AbsolutePath(path.to_owned()));
            }
            // This should never fail, as the only way for a Unix path to be absolute is to begin
            // at root /.
            path.strip_prefix("/")
                .expect("internal path processing error")
        } else {
            path
        };

        let normalized =
            normalize_relative(utf_path.as_str()).map_err(|source| PathError::UnsafePath {
                path: utf_path.as_str().into(),
                source,
            })?;

        let mut final_path = self.root_path().to_owned();
        for component in normalized.as_str().split('/') {
            final_path.push(component);
        }
        Ok(final_path)
    }

    // TODO: Do we iterate over index or checksum or both?
    pub fn get_source_paths(&self) -> Result<HashSet<Utf8PathBuf>, LocalSrcError> {
        let mut result = HashSet::new();

        match self.get_project()? {
            None => {}
            Some(ProjectDefinition::Toml(project)) => {
                for path in project.sources.iter() {
                    result.insert(self.get_source_path(path)?);
                }
            }
            Some(ProjectDefinition::Json { info: _, meta }) => {
                for path in meta.source_paths(true) {
                    result.insert(self.get_source_path(path)?);
                }
            }
        }

        Ok(result)
    }

    pub fn temporary_from_project<Pr: ProjectRead>(
        project: &Pr,
    ) -> Result<
        (
            camino_tempfile::Utf8TempDir,
            Self,
            InterchangeProjectInfoRaw,
            InterchangeProjectMetadataRaw,
        ),
        CloneError<Pr::Error, LocalSrcError>,
    > {
        let tmp = camino_tempfile::tempdir().map_err(FsIoError::MkTempDir)?;
        let mut tmp_project = Self::new_access(wrapfs::canonicalize(tmp.path())?, None);

        let (info, meta) = clone_project(project, &mut tmp_project, true)?;

        Ok((tmp, tmp_project, info, meta))
    }

    pub fn set_index(&mut self, new_index: IndexMap<String, String>) -> Result<(), LocalSrcError> {
        let mut meta = match self.get_project()? {
            None => return Err(LocalSrcError::MissingMeta),
            Some(ProjectDefinition::Toml(_)) => {
                return Err(LocalSrcError::JsonWriteToTomlProject {
                    dir: self.project_path.clone(),
                    file: META_JSON_FILE,
                });
            }
            Some(ProjectDefinition::Json { info: _, meta }) => meta,
        };
        meta.index = Some(new_index);
        self.put_meta(&meta, true)?;
        Ok(())
    }
}

/// Error of [`LocalSrcProject::edit_info_document`]: either the project
/// could not be read or written, or the edit itself failed.
#[derive(Debug, Error)]
pub enum EditInfoError<E> {
    #[error(transparent)]
    Project(#[from] LocalSrcError),
    #[error(transparent)]
    Edit(E),
}

impl LocalSrcProject {
    /// Read `.project.json` as a JSON document, apply `edit`, and write the
    /// document back pretty-printed with a trailing newline, but only if
    /// `edit` changed it.
    ///
    /// Every key the model does not know and the document's key order are
    /// kept (`serde_json` is built with `preserve_order`). The whole file is
    /// serialized to memory before anything is written, so a failing edit or
    /// serialization leaves the file untouched.
    pub fn edit_info_document<F, T, E>(&mut self, edit: F) -> Result<T, EditInfoError<E>>
    where
        F: FnOnce(&mut serde_json::Value) -> Result<T, E>,
    {
        let project_json_path = self.info_path();

        let text = wrapfs::read_to_string(&project_json_path).map_err(LocalSrcError::from)?;
        let mut doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            LocalSrcError::from(ProjectDeserializationError::new(&project_json_path, e))
        })?;
        let before = doc.clone();

        let result = edit(&mut doc).map_err(EditInfoError::Edit)?;
        if doc == before {
            return Ok(result);
        }

        let mut buf = Vec::new();
        serde_json::to_writer_pretty(&mut buf, &doc).map_err(|e| {
            LocalSrcError::from(ProjectSerializationError::new(
                format!("failed to serialize and write project info to `{project_json_path}`"),
                e,
            ))
        })?;
        buf.push(b'\n');
        wrapfs::write(&project_json_path, buf).map_err(LocalSrcError::from)?;

        Ok(result)
    }
}

impl LocalSrcProject {
    /// Read `model.toml`, resolved against the workspace this project is
    /// a member of.
    fn read_model_toml(&self) -> Result<ResolvedProject, LocalSrcError> {
        let text = self.model_toml_text()?;
        Ok(kpar_view::resolved_of_text_in(
            MODEL_TOML_FILE,
            &text,
            self.workspace.as_deref(),
        )?)
    }

    /// [`ProjectRead::get_project`], taking `model_toml` as the read of the
    /// project's `model.toml` when one is given and the project has one.
    pub fn get_project_parsed(
        &self,
        model_toml: Option<ParsedModelToml>,
    ) -> Result<Option<ProjectDefinition>, LocalSrcError> {
        match detect_format(&self.project_path)? {
            // Refused before either manifest is read.
            ProjectFormat::Both => {
                return Err(LocalSrcError::AmbiguousFormat {
                    dir: self.project_path.clone(),
                    json: json_manifest(&self.project_path)?.unwrap_or(PROJECT_JSON_FILE),
                });
            }
            ProjectFormat::Toml => {
                let resolved = match model_toml {
                    Some(parsed) => kpar_view::resolved_of_parsed(
                        MODEL_TOML_FILE,
                        parsed.manifest,
                        self.workspace.as_deref(),
                    )?,
                    None => self.read_model_toml()?,
                };
                return Ok(Some(ProjectDefinition::Toml(Box::new(resolved))));
            }
            ProjectFormat::Json | ProjectFormat::Neither => {}
        }

        let info_json_path = self.info_path();
        let meta_json_path = self.meta_path();

        // Metadata first: it carries the metamodel that errors about the
        // other document may name.
        let meta_json = if meta_json_path.exists() {
            let partial: InterchangeProjectMetadataPartial =
                serde_json::from_reader(wrapfs::File::open(&meta_json_path)?)
                    .map_err(|e| ProjectDeserializationError::new(META_JSON_FILE, e))?;
            Some(validate_manifest(
                META_JSON_FILE,
                partial.validate(META_JSON_FILE, None),
            )?)
        } else {
            None
        };
        let metamodel = meta_json
            .as_ref()
            .and_then(|meta: &InterchangeProjectMetadataRaw| meta.metamodel.as_deref());

        let info_json = if info_json_path.exists() {
            let partial: InterchangeProjectInfoPartial =
                serde_json::from_reader(wrapfs::File::open(&info_json_path)?)
                    .map_err(|e| ProjectDeserializationError::new(PROJECT_JSON_FILE, e))?;
            let info = validate_manifest(
                PROJECT_JSON_FILE,
                partial.validate(PROJECT_JSON_FILE, metamodel),
            )?;
            Some(info)
        } else {
            None
        };

        match (info_json, meta_json) {
            (Some(info), Some(meta)) => Ok(Some(ProjectDefinition::Json { info, meta })),
            (None, None) => Ok(None),
            (Some(_), None) => Err(LocalSrcError::MissingInfoMeta {
                missing: "`.meta.json`",
            }),
            (None, Some(_)) => Err(LocalSrcError::MissingInfoMeta {
                missing: "`.project.json`",
            }),
        }
    }

    /// The project's hash, which only a json project has.
    fn project_checksum(&self) -> Result<String, LocalSrcError> {
        if let Some(checksum) = &self.expected_checksum {
            return Ok(checksum.clone());
        }
        if detect_format(&self.project_path)? == ProjectFormat::Toml {
            return Err(LocalSrcError::TomlProjectHasNoChecksum {
                dir: self.project_path.clone(),
            });
        }
        self.checksum_canonical_hex()
            .map_err(|e| match e {
                CanonicalizationError::ProjectRead(e) => e,
                CanonicalizationError::FileRead(path, error) => {
                    LocalSrcError::Io(FsIoError::ReadFile(String::from(path).into(), error).into())
                }
            })?
            .ok_or(LocalSrcError::MissingInfoMeta {
                missing: NO_MANIFEST,
            })
    }

    fn model_toml_text(&self) -> Result<String, LocalSrcError> {
        Ok(wrapfs::read_to_string(
            self.project_path.join(MODEL_TOML_FILE),
        )?)
    }

    /// Refuse to write a json manifest into a `model.toml` project.
    ///
    /// Without the guard the write would leave a directory holding both
    /// manifests, which `get_project` refuses as ambiguous.
    fn refuse_json_write(&self, file: &'static str) -> Result<(), LocalSrcError> {
        if detect_format(&self.project_path)? == ProjectFormat::Toml {
            return Err(LocalSrcError::JsonWriteToTomlProject {
                dir: self.project_path.clone(),
                file,
            });
        }
        Ok(())
    }
}

impl ProjectMut for LocalSrcProject {
    fn put_info(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        self.refuse_json_write(PROJECT_JSON_FILE)?;
        let project_json_path = self.info_path();

        if !overwrite && project_json_path.exists() {
            return Err(LocalSrcError::AlreadyExists(
                "`.project.json` already exists".to_owned(),
            ));
        }

        let mut file = wrapfs::File::create(&project_json_path)?;
        serde_json::to_writer_pretty(&mut file, info).map_err(|e| {
            ProjectSerializationError::new(
                format!("failed to serialize and write project info to `{project_json_path}`"),
                e,
            )
        })?;
        file.write(b"\n")
            .map_err(|e| FsIoError::WriteFile(project_json_path, e))?;

        Ok(())
    }

    fn put_meta(
        &mut self,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        self.refuse_json_write(META_JSON_FILE)?;
        let meta_json_path = self.meta_path();
        if !overwrite && meta_json_path.exists() {
            return Err(LocalSrcError::AlreadyExists(
                "'.meta.json' already exists".to_owned(),
            ));
        }

        let mut file = wrapfs::File::create(&meta_json_path)?;
        serde_json::to_writer_pretty(&mut file, meta).map_err(|e| {
            ProjectSerializationError::new(
                format!("failed to serialize and write project metadata to `{meta_json_path}`"),
                e,
            )
        })?;
        file.write(b"\n")
            .map_err(|e| FsIoError::WriteFile(meta_json_path, e))?;

        Ok(())
    }

    fn put_manifest_document(
        &mut self,
        document: &crate::manifest::edit::DocumentMut,
        overwrite: bool,
    ) -> Result<(), crate::project::ManifestWriteError<LocalSrcError>> {
        use crate::project::ManifestWriteError;

        let format = detect_format(&self.project_path)
            .map_err(|e| ManifestWriteError::Project(LocalSrcError::from(e)))?;
        // Only a json project is refused: a directory already holding both
        // is unreadable either way.
        if format == ProjectFormat::Json {
            return Err(ManifestWriteError::Project(
                LocalSrcError::TomlWriteToJsonProject {
                    dir: self.project_path.clone(),
                },
            ));
        }

        let path = self.project_path.join(MODEL_TOML_FILE);
        if !overwrite && path.exists() {
            return Err(ManifestWriteError::Project(LocalSrcError::AlreadyExists(
                format!("`{MODEL_TOML_FILE}` already exists"),
            )));
        }

        let text = document.to_string();
        crate::manifest::edit::check(MODEL_TOML_FILE, &text)
            .map_err(|e| ManifestWriteError::Project(LocalSrcError::Deserialize(e)))?;
        wrapfs::write(&path, text)
            .map_err(|e| ManifestWriteError::Project(LocalSrcError::from(e)))?;

        Ok(())
    }

    fn write_source<P: AsRef<Utf8UnixPath>, R: Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        let source_path = self.get_source_path(path)?;

        if !overwrite && source_path.exists() {
            return Err(LocalSrcError::AlreadyExists(format!(
                "`{source_path}` already exists"
            )));
        }

        if let Some(parents) = source_path.parent() {
            wrapfs::create_dir_all(parents)?;
        }

        std::io::copy(source, &mut wrapfs::File::create(&source_path)?)
            .map_err(|e| FsIoError::WriteFile(source_path, e))?;

        Ok(())
    }
}

/// What a directory with no manifest is missing.
const NO_MANIFEST: &str = "`model.toml`, or `.project.json` and `.meta.json`";

#[derive(Error, Debug)]
pub enum LocalSrcError {
    #[error("{0}")]
    AlreadyExists(String),
    #[error("project is missing metadata file `.meta.json`")]
    MissingMeta,
    /// The project has no manifest to read, or a json project lacks one
    /// of its two documents. `missing` names what is missing.
    #[error("project is missing {missing}")]
    MissingInfoMeta { missing: &'static str },
    /// A project source pinned by checksum is a `model.toml` project,
    /// which has no project checksum.
    #[error(
        "`{dir}` is a `model.toml` project, which has no project checksum;\n\
        {SP:>8} it can only be used as an editable project",
        SP = ' '
    )]
    TomlProjectHasNoChecksum { dir: Utf8PathBuf },
    /// A directory holding both manifest kinds. The message carries no
    /// `sysand migrate` hint.
    #[error(
        "`{dir}` has both `model.toml` and `{json}`;\n\
        {SP:>8} it is unclear whether it is a non-zipped KPAR or a development project",
        SP = ' '
    )]
    AmbiguousFormat {
        dir: Utf8PathBuf,
        json: &'static str,
    },
    /// A command that writes a json manifest was pointed at a
    /// `model.toml` project. Writing one would leave the directory
    /// holding both manifests, which nothing can read.
    #[error(
        "`{dir}` is a `model.toml` project;\n\
        {SP:>8} `{file}` cannot be written into one",
        SP = ' '
    )]
    JsonWriteToTomlProject {
        dir: Utf8PathBuf,
        file: &'static str,
    },
    /// The mirror: a command that writes a `model.toml` was pointed at a
    /// json project. The directory would then hold both manifests, which
    /// nothing can read.
    #[error(
        "`{dir}` is a `.project.json` project;\n\
        {SP:>8} `{MODEL_TOML_FILE}` cannot be written into one (see `sysand migrate`)",
        SP = ' '
    )]
    TomlWriteToJsonProject { dir: Utf8PathBuf },
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Serialize(#[from] ProjectSerializationError),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(
        "cannot construct a relative path from the workspace/project
        directory to one of its dependencies' directory"
    )]
    ImpossibleRelativePath(#[from] RelativizePathError),
}

impl From<FsIoError> for LocalSrcError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

#[derive(Error, Debug)]
pub enum UnixPathError {
    #[error("path `{0}`\n  is outside the project directory")]
    PathOutsideProject(Utf8PathBuf),
    #[error(transparent)]
    Resolve(#[from] ResolvePathError),
}

#[derive(Error, Debug)]
pub enum PathError {
    #[error("path `{path}` is unsafe")]
    UnsafePath {
        path: Box<str>,
        #[source]
        source: NormalizeError,
    },
    #[error("path `{0}` is absolute")]
    AbsolutePath(typed_path::Utf8UnixPathBuf),
}

impl ProjectRead for LocalSrcProject {
    type Error = LocalSrcError;

    fn project_root(&self) -> Option<&Utf8Path> {
        Some(&self.project_path)
    }

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        Ok(detect_format(&self.project_path)?)
    }

    fn manifest_document(
        &self,
    ) -> Result<crate::manifest::edit::DocumentMut, ManifestError<Self::Error>> {
        if self.format().map_err(ManifestError::Project)? != ProjectFormat::Toml {
            return Err(ManifestError::NoTomlManifest);
        }
        let text = self.model_toml_text().map_err(ManifestError::Project)?;
        crate::manifest::edit::validated(MODEL_TOML_FILE, &text)
            .map_err(|e| ManifestError::Project(LocalSrcError::Deserialize(e)))
    }

    fn get_project(&self) -> Result<Option<ProjectDefinition>, LocalSrcError> {
        self.get_project_parsed(None)
    }

    type SourceReader<'a> = File;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, LocalSrcError> {
        let source_path = self.get_source_path(path)?;

        let f = wrapfs::File::open(&source_path)?;

        Ok(f)
    }

    // TODO: should `sources` be decoupled from `ProjectRead`? It requires knowing additional
    // integrity details, such as kpar/project checksums, which therefore have to be stored
    // inside the ProjectRead implementer, as there is no other wrapper used. This needs a
    // consistent design for all project types. E.g. should they contain any checksums? when
    // should they be verified to match the project?
    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        let checksum = self.project_checksum()?;
        let src_path = ctx
            .lockfile_path(self.nominal_path.as_ref(), &self.project_path)?
            .into_string()
            .into();
        Ok(vec![Source::LocalSrc { src_path, checksum }])
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // A path names one project.
        false
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        Ok(ProjectChecksum::Project(self.project_checksum()?))
    }
}

#[cfg(test)]
#[path = "./local_src_tests.rs"]
mod tests;
