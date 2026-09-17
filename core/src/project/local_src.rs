// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    cell::OnceCell,
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
    manifest::{
        MODEL_TOML_FILE,
        kpar_view::{self, ViewError},
        project::ResolvedProject,
    },
    model::{
        InterchangeProjectInfoPartial, InterchangeProjectInfoRaw,
        InterchangeProjectMetadataPartial, InterchangeProjectMetadataRaw, META_JSON_FILE,
        PROJECT_JSON_FILE,
    },
    project::{
        ManifestError, ProjectMut, ProjectRead,
        utils::{RelativizePathError, ToPathBuf as _, ToUnixPathBuf as _, relativize_path, wrapfs},
    },
};

use super::{
    CanonicalizationError, ProjectChecksum,
    utils::{FsIoError, ProjectDeserializationError, ProjectSerializationError, validate_manifest},
};

pub use crate::project::ProjectFormat;

/// Which manifests `dir` holds.
///
/// `wrapfs::is_file` rather than `Utf8Path::exists`, so that an IO error
/// is reported instead of swallowed and a *directory* named
/// `.project.json` is not mistaken for a manifest.
pub fn detect_format(dir: &Utf8Path) -> Result<ProjectFormat, Box<FsIoError>> {
    let toml = wrapfs::is_file(dir.join(MODEL_TOML_FILE))?;
    Ok(match (toml, json_manifest(dir)?.is_some()) {
        (true, true) => ProjectFormat::Both,
        (true, false) => ProjectFormat::Toml,
        (false, true) => ProjectFormat::Json,
        (false, false) => ProjectFormat::Neither,
    })
}

/// The json manifest `dir` holds, `.project.json` first when it holds
/// both. Only a message needs to know which one, and naming a file that
/// is not there would send the reader looking for it.
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
    /// The workspace this project is a member of, if a caller said so.
    ///
    /// A member's manifest may carry inheritance markers, and resolving
    /// with `None` refuses every one of them. Nothing goes looking for a
    /// table: most `LocalSrcProject`s have no workspace and must keep
    /// resolving with `None`, and a project that walked the filesystem
    /// from inside a manifest parse would be a different thing.
    ///
    /// `Arc` because a `WorkspaceTable` is several hundred bytes and
    /// this struct is a variant of `FileResolverProject`, for the same
    /// reason the view below is boxed.
    workspace: Option<std::sync::Arc<crate::manifest::workspace_table::WorkspaceTable>>,
    /// The pair a `model.toml` project answers with, computed once.
    ///
    /// Building it parses the manifest and then reads, hashes and
    /// extracts symbols from every source, while `checksum_canonical_hex`
    /// alone asks for the project twice (`get_info` and `get_meta`). It
    /// is filled on the toml path only: on the json path the writers of
    /// [`ProjectMut`] and [`LocalSrcProject::set_index`] change the files
    /// underneath it, and a cached pair would then contradict them.
    ///
    /// Boxed because a `LocalSrcProject` is a variant of
    /// `FileResolverProject`, and the pair inline makes that enum several
    /// hundred bytes larger than its other variant.
    view: OnceCell<Box<ViewPair>>,
}

/// The `.project.json`/`.meta.json` pair, as [`ProjectRead::get_project`]
/// answers with it.
type ViewPair = (
    Option<InterchangeProjectInfoRaw>,
    Option<InterchangeProjectMetadataRaw>,
);

impl LocalSrcProject {
    /// Access the project. No invariants will be checked. If `nominal_path` is provided,
    /// `sources()` will work.
    pub fn new_access(path: impl Into<Utf8PathBuf>, nominal_path: Option<Utf8UnixPathBuf>) -> Self {
        Self {
            nominal_path,
            project_path: path.into(),
            expected_checksum: None,
            workspace: None,
            view: OnceCell::new(),
        }
    }

    /// Like [`Self::new_access`], for a caller that already knows the
    /// canonical project checksum — an environment that recorded the checksum
    /// of what it installed, say. Knowing it spares `sources()` a hash of the
    /// whole project tree to arrive at the same value; a caller that does not
    /// have one wants [`Self::new_access`].
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

    /// The same project, resolving its manifest against `table`.
    ///
    /// For a resolver that has discovered its target is a member of the
    /// workspace it was given -- the table still arrives from outside,
    /// so the invariant above holds: nothing here goes looking for one.
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
        let root_path = self.root_path();
        let project_path = wrapfs::canonicalize_raw(root_path)
            .map_err(|e| UnixPathError::Canonicalize(root_path.to_owned(), e))?;

        let path = relativize_path_in(&path, project_path)
            .ok_or_else(|| UnixPathError::PathOutsideProject(path.to_path_buf()))?;

        Ok(path.to_unix_path_buf())
    }

    pub fn get_source_path<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Utf8PathBuf, PathError> {
        let path = path.as_ref();
        let utf_path = if path.is_absolute() {
            if !cfg!(feature = "lenient_checks") {
                return Err(PathError::AbsolutePath(path.to_owned()));
            }
            // This should never fail, as the only way for a Unix path to be absolute is to begin
            // at root /.
            path.strip_prefix("/")
                .expect("internal path processing error")
        } else {
            path
        };

        assert!(utf_path.is_relative());

        let mut final_path = self.root_path().to_owned();
        let mut added_components = 0;
        for component in utf_path.components() {
            match component {
                typed_path::Utf8UnixComponent::RootDir => {
                    unreachable!("root component in a relative path: {utf_path}")
                }
                typed_path::Utf8UnixComponent::CurDir => {}
                typed_path::Utf8UnixComponent::ParentDir => {
                    if added_components > 0 {
                        assert!(final_path.pop());
                        added_components -= 1;
                    } else {
                        return Err(PathError::UnsafePath(
                            utf_path.as_str().into(),
                            typed_path::CheckedPathError::PathTraversalAttack,
                        ));
                    }
                }
                typed_path::Utf8UnixComponent::Normal(component_str) => {
                    final_path.push(component_str);
                    added_components += 1;
                }
            }
        }
        Ok(final_path)
    }

    // TODO: Do we iterate over index or checksum or both?
    pub fn get_source_paths(&self) -> Result<HashSet<Utf8PathBuf>, LocalSrcError> {
        let mut result = HashSet::new();

        if let Some(meta) = self.get_meta()? {
            for path in meta.source_paths(true) {
                let source_path = self.get_source_path(path)?;
                result.insert(source_path);
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
        let mut meta = self.get_meta()?.ok_or(LocalSrcError::MissingMeta)?;
        meta.index = new_index;
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
    /// document back in the byte shape `put_info` produces (pretty-printed,
    /// trailing newline) — but only if `edit` changed it. An unchanged
    /// document does not touch the file at all.
    ///
    /// Editing the document rather than the typed model keeps every key the
    /// model does not know and the document's key order intact
    /// (`serde_json` is built with `preserve_order`). The whole file is
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
    /// The pair a `model.toml` project answers with, read once.
    ///
    /// A failure is not cached: `OnceCell::get_or_try_init` is unstable,
    /// and these errors are not `Clone` -- the same reason
    /// `LocalKParProject::ensure_initialized` gives.
    fn toml_view(&self) -> Result<&ViewPair, LocalSrcError> {
        if let Some(view) = self.view.get() {
            return Ok(view);
        }
        let view = Box::new(self.read_model_toml()?);
        Ok(self.view.get_or_init(|| view))
    }

    /// Read `model.toml` and synthesize the KPAR view of it.
    ///
    /// The reading, parsing and mapping all live in `kpar_view`, which
    /// is free of Cargo features, so that a store without a filesystem
    /// answers with the same pair. What is left here is the two things
    /// only a directory can supply: the manifest's text, and a way to
    /// read a source file safely from inside the project.
    fn read_model_toml(&self) -> Result<ViewPair, LocalSrcError> {
        let text = self.model_toml_text()?;
        let (info, meta) =
            kpar_view::of_model_toml_in(&text, self.workspace.as_deref(), |source| {
                let source_path = self.get_source_path(source)?;
                Ok(wrapfs::read_to_string(&source_path)?)
            })
            .map_err(|error| match error {
                ViewError::Manifest(error) => LocalSrcError::Deserialize(error),
                ViewError::Read(error) => error,
                ViewError::Symbols(error) => LocalSrcError::Symbols(error),
            })?;

        Ok((Some(info), Some(meta)))
    }

    fn model_toml_text(&self) -> Result<String, LocalSrcError> {
        Ok(wrapfs::read_to_string(
            self.project_path.join(MODEL_TOML_FILE),
        )?)
    }

    /// Which manifest this project would have to have.
    ///
    /// A directory with none of the three is missing all of them; one
    /// that has a json manifest is missing the other json one, and
    /// saying `model.toml` there would send the reader to write the
    /// wrong file. A toml project cannot get here at all, since its view
    /// always has both documents.
    fn missing_manifest(&self) -> LocalSrcError {
        const JSON: &str = "`.project.json` and/or `.meta.json` files";
        let missing = match detect_format(&self.project_path) {
            Ok(ProjectFormat::Neither) => "`model.toml`, or `.project.json` and/or `.meta.json`",
            Ok(_) => JSON,
            // The format could not be read, so the honest answer is the
            // one that names everything a manifest could be.
            Err(_) => "`model.toml`, or `.project.json` and/or `.meta.json`",
        };
        LocalSrcError::MissingInfoMeta { missing }
    }

    /// Refuse to write a json manifest into a `model.toml` project.
    ///
    /// A json manifest reaches the filesystem through `put_info` or
    /// `put_meta`, so the guard sits there. Without it the write would
    /// succeed and leave a directory holding both manifests, which is
    /// the ambiguity `get_project` then refuses: the project would be
    /// unreadable and nothing would say why.
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
        // The mirror of `refuse_json_write`, and as narrow: that one
        // fires on `Toml` alone, so this one fires on `Json` alone.
        // Writing into a directory that already holds both changes
        // nothing about why it is unreadable.
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

        wrapfs::write(&path, document.to_string())
            .map_err(|e| ManifestWriteError::Project(LocalSrcError::from(e)))?;

        // The cached view is of the manifest that was there.
        self.view.take();
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

#[derive(Error, Debug)]
pub enum LocalSrcError {
    #[error("{0}")]
    AlreadyExists(String),
    #[error("project is missing metadata file `.meta.json`")]
    MissingMeta,
    /// The project has no manifest to read. `missing` names the files
    /// that would be one, which depends on the format: naming
    /// `model.toml` at a json project that is merely missing one of its
    /// two documents would point at the wrong fix.
    #[error("project is missing {missing}")]
    MissingInfoMeta { missing: &'static str },
    /// model-toml.md "Mixed manifests". The message carries no
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
    /// A source file listed in `sources` could not be indexed.
    #[error(transparent)]
    Symbols(#[from] crate::symbols::SymbolsError),
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
        directory to one of its dependencies' directory:\n\
        {0}"
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
    #[error("failed to canonicalize\n  `{0}`:\n  {1}")]
    Canonicalize(Utf8PathBuf, std::io::Error),
}

#[derive(Error, Debug)]
pub enum PathError {
    #[error("path `{0}` is unsafe: {1}")]
    UnsafePath(Utf8PathBuf, typed_path::CheckedPathError),
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

    fn manifest(&self) -> Result<ResolvedProject, ManifestError<Self::Error>> {
        if self.format().map_err(ManifestError::Project)? != ProjectFormat::Toml {
            return Err(ManifestError::NoTomlManifest);
        }
        let text = self.model_toml_text().map_err(ManifestError::Project)?;
        kpar_view::resolved_of_text_in(MODEL_TOML_FILE, &text, self.workspace.as_deref())
            .map_err(|error| ManifestError::Project(LocalSrcError::Deserialize(error)))
    }

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        LocalSrcError,
    > {
        match detect_format(&self.project_path)? {
            // model-toml.md "Mixed manifests": a directory holding both
            // kinds of manifest is refused before either is read.
            ProjectFormat::Both => {
                return Err(LocalSrcError::AmbiguousFormat {
                    dir: self.project_path.clone(),
                    json: json_manifest(&self.project_path)?.unwrap_or(PROJECT_JSON_FILE),
                });
            }
            ProjectFormat::Toml => return self.toml_view().cloned(),
            ProjectFormat::Json | ProjectFormat::Neither => {}
        }

        let info_json_path = self.info_path();
        let meta_json_path = self.meta_path();

        // The metadata is read first because it carries the metamodel,
        // which a message about the other document may need to name.
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

        Ok((info_json, meta_json))
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
        let checksum = match &self.expected_checksum {
            Some(c) => c.clone(),
            None => self
                .checksum_canonical_hex()
                .map_err(|e| match e {
                    CanonicalizationError::ProjectRead(e) => e,
                    CanonicalizationError::FileRead(path, error) => LocalSrcError::Io(
                        FsIoError::ReadFile(String::from(path).into(), error).into(),
                    ),
                })?
                .ok_or_else(|| self.missing_manifest())?,
        };
        let src_path = if let Some(np) = self.nominal_path.as_ref() {
            np.as_str().into()
        } else if let Some(root) = ctx.sources_root() {
            relativize_path(&self.project_path, &root)?
                .into_string()
                .into()
        } else {
            panic!("`LocalSrcProject` without `nominal_path` does not have any project sources");
        };
        Ok(vec![Source::LocalSrc { src_path, checksum }])
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // A path names one project. Note that an environment hands back one of
        // these per installed version -- see the TODO on
        // `LocalDirectoryEnvironment::get_project_storage`.
        false
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        match self.checksum_canonical_hex() {
            Ok(c) => match c {
                Some(c) => Ok(ProjectChecksum::Project(c)),
                None => Err(self.missing_manifest()),
            },
            Err(e) => match e {
                CanonicalizationError::ProjectRead(e) => Err(e),
                CanonicalizationError::FileRead(path, error) => Err(LocalSrcError::Io(
                    FsIoError::ReadFile(String::from(path).into(), error).into(),
                )),
            },
        }
    }
}

/// Tries to canonicalize the (longest possible) prefix of a path.
/// Useful if you have /path/to/file/that/does/not/exist
/// but where some prefix, say, /path/to/file can be canonicalized.
fn canonicalize_prefix<P: AsRef<Utf8Path>>(path: P) -> Utf8PathBuf {
    let mut relative_part = Utf8PathBuf::new();
    let mut absolute_part = path.to_path_buf();

    loop {
        if let Ok(canonical_absolute) = wrapfs::canonicalize_raw(&absolute_part) {
            absolute_part = canonical_absolute;
            break;
        }

        match (absolute_part.parent(), absolute_part.file_name()) {
            (Some(absolute_part_parent), Some(absolute_part_file)) => {
                relative_part = Utf8Path::new(absolute_part_file).join(relative_part);
                absolute_part = absolute_part_parent.to_path_buf();
            }
            _ => {
                break;
            }
        }
    }

    absolute_part.push(relative_part);
    absolute_part
}

fn relativize_path_in<P: AsRef<Utf8Path>, Q: AsRef<Utf8Path>>(
    path: P,
    relative_to: Q,
) -> Option<Utf8PathBuf> {
    let path = if path.as_ref().is_absolute() {
        canonicalize_prefix(path)
    } else {
        let path = camino::absolute_utf8(path.as_ref()).ok()?;
        canonicalize_prefix(path)
    };

    path.strip_prefix(canonicalize_prefix(relative_to))
        .ok()
        .map(camino::Utf8Path::to_path_buf)
}

#[cfg(test)]
#[path = "./local_src_tests.rs"]
mod tests;
