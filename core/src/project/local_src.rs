// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::paths::{
    NormalizeError, RelativizePathError, ResolvePathError, normalize_relative, relativize_path_in,
};
use std::{
    fs::File,
    io::{ErrorKind, Read, Write as _},
};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use indexmap::IndexMap;

use crate::{
    context::ProjectContext,
    lock::Source,
    manifest::{
        MODEL_TOML_FILE, ParsedModelToml,
        edit::{self, DocumentMut},
        kpar_view,
        project::ResolvedProject,
    },
    model::{
        InterchangeProjectInfoPartial, InterchangeProjectInfoRaw,
        InterchangeProjectMetadataPartial, InterchangeProjectMetadataRaw, META_JSON_FILE,
        PROJECT_JSON_FILE, PartialManifest,
    },
    project::{
        FormatError, ManifestError, ProjectDefinition, ProjectMut, ProjectRead,
        utils::{ToPathBuf as _, wrapfs},
    },
};

use super::{
    CanonicalizationError, ProjectChecksum,
    utils::{
        FsIoError, ProjectDeserializationError, ProjectSerializationError, read_json_manifest,
    },
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
    /// The manifest of a workspace member, resolved by its workspace.
    ///
    /// A member's manifest may carry inheritance markers, which resolve only
    /// against the workspace, so a member's `model.toml` is read through
    /// this and never resolved here. It must match the `model.toml` on
    /// disk: whoever edits the file replaces it.
    member_manifest: Option<Box<ResolvedProject>>,
}

impl LocalSrcProject {
    /// Access the project. No invariants will be checked. If `nominal_path` is provided,
    /// `sources()` will work.
    pub fn new_access(path: impl Into<Utf8PathBuf>, nominal_path: Option<Utf8UnixPathBuf>) -> Self {
        Self {
            nominal_path,
            project_path: path.into(),
            expected_checksum: None,
            member_manifest: None,
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
    pub fn path(&self) -> &Utf8Path {
        &self.project_path
    }

    /// The same project, read as a workspace member whose manifest its
    /// workspace resolved to `manifest`.
    #[must_use]
    pub fn with_member(mut self, manifest: ResolvedProject) -> Self {
        self.member_manifest = Some(Box::new(manifest));
        self
    }

    /// [`LocalSrcProject::new_access`] for a workspace member whose
    /// manifest its workspace resolved to `manifest`.
    pub fn new_member(
        path: impl Into<Utf8PathBuf>,
        nominal_path: Option<Utf8UnixPathBuf>,
        manifest: ResolvedProject,
    ) -> Self {
        Self::new_access(path, nominal_path).with_member(manifest)
    }

    /// Replace the member's manifest, after its `model.toml` changed.
    pub fn set_member_manifest(&mut self, manifest: ResolvedProject) {
        self.member_manifest = Some(Box::new(manifest));
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

    pub fn set_index(&mut self, new_index: IndexMap<String, String>) -> Result<(), LocalSrcError> {
        let mut meta = match self.get_project()? {
            None => return Err(LocalSrcError::MissingMeta),
            Some(ProjectDefinition::Toml(_)) => {
                return Err(self.format_error(FormatError::JsonWriteToToml(META_JSON_FILE)));
            }
            Some(ProjectDefinition::Json { info: _, meta }) => meta,
        };
        meta.index = Some(new_index);
        self.put_meta(&meta, true)?;
        Ok(())
    }
}

impl LocalSrcProject {
    /// Write the json documents `after` over what is on disk, `.meta.json`
    /// first. A document equal to its `before` is not written.
    ///
    /// Keys the model does not know, and the key order of the file, are
    /// kept: only the keys the model writes are replaced. Each file is
    /// replaced atomically; a failure between the two leaves the first
    /// written.
    pub fn write_json_documents(
        &self,
        before: (&InterchangeProjectInfoRaw, &InterchangeProjectMetadataRaw),
        after: (&InterchangeProjectInfoRaw, &InterchangeProjectMetadataRaw),
    ) -> Result<(), LocalSrcError> {
        self.rewrite_json(META_JSON_FILE, before.1, after.1)?;
        self.rewrite_json(PROJECT_JSON_FILE, before.0, after.0)
    }

    fn rewrite_json<T: serde::Serialize>(
        &self,
        file: &'static str,
        before: &T,
        after: &T,
    ) -> Result<(), LocalSrcError> {
        let serialization =
            |e| ProjectSerializationError::new(format!("failed to serialize `{file}`"), e);
        let before = serde_json::to_value(before).map_err(serialization)?;
        let after = serde_json::to_value(after).map_err(serialization)?;
        if before == after {
            return Ok(());
        }
        detect_format(&self.project_path)?
            .check_json_write(file)
            .map_err(|problem| self.format_error(problem))?;
        let path = self.project_path.join(file);
        let text = wrapfs::read_to_string(&path)?;
        let mut document: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| ProjectDeserializationError::new(&path, e))?;
        overlay_json(&mut document, &before, &after);
        let mut buf = serde_json::to_vec_pretty(&document).map_err(serialization)?;
        buf.push(b'\n');
        wrapfs::write_atomic(&path, buf)?;
        Ok(())
    }
}

/// Make `document` carry `after` where it carried `before`.
///
/// What is the same in `before` and `after` is left as `document` has it.
/// In objects, keys `after` lacks are removed and the others are set, and
/// keys that are in neither are kept. Arrays of the same length are
/// overlaid element by element.
fn overlay_json(
    document: &mut serde_json::Value,
    before: &serde_json::Value,
    after: &serde_json::Value,
) {
    use serde_json::Value;

    if before == after {
        return;
    }
    match (&mut *document, before, after) {
        (Value::Object(document), Value::Object(before), Value::Object(after)) => {
            for key in before.keys().filter(|key| !after.contains_key(*key)) {
                document.shift_remove(key);
            }
            for (key, value) in after {
                match (document.get_mut(key), before.get(key)) {
                    (Some(current), Some(previous)) => overlay_json(current, previous, value),
                    _ => {
                        document.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (Value::Array(document), Value::Array(before), Value::Array(after))
            if document.len() == before.len() && before.len() == after.len() =>
        {
            for ((current, previous), value) in document.iter_mut().zip(before).zip(after) {
                overlay_json(current, previous, value);
            }
        }
        (document, _, after) => *document = after.clone(),
    }
}

/// The json documents as reading them back would validate them.
///
/// Validation warnings are dropped: they were logged when the documents
/// were read.
pub fn validated_json_documents(
    info: &InterchangeProjectInfoRaw,
    meta: &InterchangeProjectMetadataRaw,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), ProjectDeserializationError>
{
    fn revalidate<T: PartialManifest>(
        document: &impl serde::Serialize,
        metamodel: Option<&str>,
    ) -> Result<T::Validated, ProjectDeserializationError> {
        let value = serde_json::to_value(document)
            .map_err(|e| ProjectDeserializationError::new(T::FILE, e))?;
        let partial: T = serde_json::from_value(value)
            .map_err(|e| ProjectDeserializationError::new(T::FILE, e))?;
        match partial.validate_manifest(metamodel) {
            Ok(validated) => Ok(validated.value),
            Err(report) => Err(ProjectDeserializationError::invalid(T::FILE, report)),
        }
    }
    let meta = revalidate::<InterchangeProjectMetadataPartial>(meta, None)?;
    let info = revalidate::<InterchangeProjectInfoPartial>(info, meta.metamodel.as_deref())?;
    Ok((info, meta))
}

impl LocalSrcProject {
    /// Read `model.toml`: the member's manifest as its workspace resolved
    /// it, or the file resolved standalone.
    fn read_model_toml(&self) -> Result<ResolvedProject, LocalSrcError> {
        if let Some(manifest) = &self.member_manifest {
            return Ok((**manifest).clone());
        }
        let text = self.model_toml_text()?;
        Ok(kpar_view::resolved_of_text(MODEL_TOML_FILE, &text)?)
    }

    /// [`ProjectRead::get_project`], taking `model_toml` as the read of the
    /// project's `model.toml` when one is given and the project has one.
    pub fn get_project_parsed(
        &self,
        model_toml: Option<ParsedModelToml>,
    ) -> Result<Option<ProjectDefinition>, LocalSrcError> {
        match detect_format(&self.project_path)? {
            // Refused before either manifest is read.
            ProjectFormat::Both => return Err(self.ambiguous_error()?),
            ProjectFormat::Toml => {
                let resolved = match model_toml {
                    Some(parsed) if self.member_manifest.is_none() => {
                        kpar_view::resolved_of_parsed(MODEL_TOML_FILE, parsed.manifest)?
                    }
                    Some(_) | None => self.read_model_toml()?,
                };
                return Ok(Some(ProjectDefinition::Toml(Box::new(resolved))));
            }
            ProjectFormat::Json | ProjectFormat::Neither => {}
        }

        // Metadata first: it carries the metamodel that errors about the
        // other document may name.
        let meta = self.read_json::<InterchangeProjectMetadataPartial>(None)?;
        let info = self.read_json::<InterchangeProjectInfoPartial>(
            meta.as_ref().and_then(|meta| meta.metamodel.as_deref()),
        )?;
        ProjectDefinition::from_json(info, meta).map_err(|problem| self.format_error(problem))
    }

    /// The json manifest document `T`, `None` when the project has no such
    /// file.
    fn read_json<T: PartialManifest>(
        &self,
        metamodel: Option<&str>,
    ) -> Result<Option<T::Validated>, LocalSrcError> {
        let path = self.project_path.join(T::FILE);
        let file = match wrapfs::File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        Ok(Some(read_json_manifest::<T>(file, metamodel)?))
    }

    /// The error of this project breaking the format rules.
    fn format_error(&self, problem: FormatError) -> LocalSrcError {
        LocalSrcError::Format {
            dir: self.project_path.clone(),
            problem,
        }
    }

    /// The error for a directory holding both `model.toml` and a json
    /// manifest.
    fn ambiguous_error(&self) -> Result<LocalSrcError, LocalSrcError> {
        Ok(self.format_error(FormatError::Ambiguous {
            json: json_manifest(&self.project_path)?.unwrap_or(PROJECT_JSON_FILE),
        }))
    }

    /// The project's hash, which only a json project has.
    fn project_checksum(&self) -> Result<String, LocalSrcError> {
        if let Some(checksum) = &self.expected_checksum {
            return Ok(checksum.clone());
        }
        if detect_format(&self.project_path)? == ProjectFormat::Toml {
            return Err(self.format_error(FormatError::TomlHasNoChecksum));
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

    /// Write `value` as the json manifest `file`, pretty-printed with a
    /// trailing newline.
    fn write_json(
        &self,
        file: &'static str,
        value: &impl serde::Serialize,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        detect_format(&self.project_path)?
            .check_json_write(file)
            .map_err(|problem| self.format_error(problem))?;
        let path = self.project_path.join(file);

        wrapfs::create_dir_all(&self.project_path)?;
        let mut buf = serde_json::to_vec_pretty(value).map_err(|e| {
            ProjectSerializationError::new(format!("failed to serialize `{path}`"), e)
        })?;
        buf.push(b'\n');
        write_file(&path, &buf, overwrite, file)?;

        Ok(())
    }
}

impl ProjectMut for LocalSrcProject {
    fn put_info(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        self.write_json(PROJECT_JSON_FILE, info, overwrite)
    }

    fn put_meta(
        &mut self,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        self.write_json(META_JSON_FILE, meta, overwrite)
    }

    fn put_manifest_document(
        &mut self,
        document: &DocumentMut,
        overwrite: bool,
    ) -> Result<(), crate::project::ManifestWriteError<LocalSrcError>> {
        self.format()?
            .check_toml_write()
            .map_err(|problem| self.format_error(problem))?;

        let path = self.project_path.join(MODEL_TOML_FILE);
        let text = edit::checked_text(MODEL_TOML_FILE, document).map_err(LocalSrcError::from)?;
        wrapfs::create_dir_all(&self.project_path).map_err(LocalSrcError::from)?;
        write_file(&path, text.as_bytes(), overwrite, MODEL_TOML_FILE)?;

        Ok(())
    }

    fn write_source<P: AsRef<Utf8UnixPath>, R: Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        let source_path = self.get_source_path(path)?;

        if let Some(parents) = source_path.parent() {
            wrapfs::create_dir_all(parents)?;
        }

        let mut file = create_file(&source_path, overwrite, source_path.as_str())?;
        std::io::copy(source, &mut file).map_err(|e| FsIoError::WriteFile(source_path, e))?;

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
    /// The project has no manifest to read. `missing` names what is
    /// missing.
    #[error("project is missing {missing}")]
    MissingInfoMeta { missing: &'static str },
    #[error("`{dir}` {problem}")]
    Format {
        dir: Utf8PathBuf,
        problem: FormatError,
    },
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
    AbsolutePath(Utf8UnixPathBuf),
}

impl ProjectRead for LocalSrcProject {
    type Error = LocalSrcError;

    fn project_root(&self) -> Option<&Utf8Path> {
        Some(&self.project_path)
    }

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        Ok(detect_format(&self.project_path)?)
    }

    fn manifest_document(&self) -> Result<DocumentMut, ManifestError<Self::Error>> {
        if self.format()? != ProjectFormat::Toml {
            return Err(ManifestError::NoTomlManifest);
        }
        let text = self.model_toml_text()?;
        Ok(edit::validated(MODEL_TOML_FILE, &text).map_err(LocalSrcError::from)?)
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

/// Writes `contents` to the file at `path`. An existing file is replaced
/// atomically when `overwrite`; otherwise it is an `AlreadyExists` error
/// naming it as `what`.
fn write_file(
    path: &Utf8Path,
    contents: &[u8],
    overwrite: bool,
    what: &str,
) -> Result<(), LocalSrcError> {
    if overwrite {
        return Ok(wrapfs::write_atomic(path, contents)?);
    }
    let mut file = create_file(path, false, what)?;
    file.write_all(contents)
        .map_err(|e| FsIoError::WriteFile(path.into(), e))?;
    Ok(())
}

/// Creates the file at `path`, truncating an existing one only when
/// `overwrite`. Otherwise an existing file is an `AlreadyExists` error naming
/// it as `what`.
fn create_file(path: &Utf8Path, overwrite: bool, what: &str) -> Result<File, LocalSrcError> {
    if overwrite {
        return Ok(wrapfs::File::create(path)?);
    }
    wrapfs::File::create_new(path).map_err(|err| {
        if err.kind() == ErrorKind::AlreadyExists {
            LocalSrcError::AlreadyExists(format!("`{what}` already exists"))
        } else {
            err.into()
        }
    })
}

#[cfg(test)]
#[path = "./local_src_tests.rs"]
mod tests;
