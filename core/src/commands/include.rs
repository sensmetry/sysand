// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::io::Read as _;

use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::{
    model::KerMlChecksumAlg,
    project::{ProjectFormat, ProjectMut, ProjectOrIOError, ProjectRead, utils::FsIoError},
    symbols::{ExtractError, Language, SymbolsError},
    utils::sha256_lowercase_hex,
};

#[derive(Error, Debug)]
pub enum IncludeError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("failed to extract symbol names from `{0}`: {1}")]
    Extract(Box<str>, ExtractError),
    #[error(
        "unknown file format of `{0}`, only SysML v2 (.sysml) and KerML (.kerml) files are supported"
    )]
    UnknownFormat(Box<str>),
    /// The two maps a `model.toml` project has no room for are computed
    /// when the KPAR is built, so the flags that control them have
    /// nothing to control.
    ///
    /// Refused here rather than in the CLI because the parameters are
    /// this function's: a binding can call it directly, and a refusal
    /// one frontend makes is not a refusal.
    #[error("`{0}` is computed when the KPAR is built and cannot be set on a `model.toml` project")]
    ComputedField(&'static str),
    /// A Syside project's KPAR has no `index`, so the flag has nothing to
    /// apply to (design/syside-metamodel.md, "Build").
    #[error(
        "`--no-index-symbols` applies to projects with a computed index; a Syside project's \
         KPAR carries `root` and `exports` instead"
    )]
    NoIndexSymbolsOnRoot,
    /// `--force-format` selects a parser for symbol extraction, and a
    /// `model.toml` project extracts none.
    #[error(
        "`force_format` has no effect on a `model.toml` project, whose symbols are computed when the KPAR is built"
    )]
    ForceFormat,
    /// Checked before the manifest is touched: a file the
    /// project's metamodel does not admit would be written into
    /// `sources` and refused by the next read.
    #[error("{0}")]
    NotAdmitted(Box<str>),
    /// The manifest could not be read, or the edit was refused.
    #[error(transparent)]
    Edit(crate::manifest::edit::EditError),
    #[error("failed to read the project's `model.toml`: {0}")]
    Manifest(Box<str>),
}

impl<ProjectError> From<FsIoError> for IncludeError<ProjectError> {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl<ProjectError> From<SymbolsError> for IncludeError<ProjectError> {
    fn from(value: SymbolsError) -> Self {
        match value {
            SymbolsError::Extract(path, error) => Self::Extract(path, error),
            SymbolsError::UnknownFormat(path) => Self::UnknownFormat(path),
        }
    }
}

impl<ProjectError> From<ProjectOrIOError<ProjectError>> for IncludeError<ProjectError> {
    fn from(value: ProjectOrIOError<ProjectError>) -> Self {
        match value {
            ProjectOrIOError::Project(error) => Self::Project(error),
            ProjectOrIOError::Io(error) => Self::Io(error),
        }
    }
}

// TODO: Add a CLI option to make the file format explicit (useful in cases
// of non-standard file extensions)
pub fn do_include<Pr: ProjectMut, I: Iterator<Item = Utf8UnixPathBuf>>(
    project: &mut Pr,
    paths: I,
    compute_checksum: bool,
    index_symbols: bool,
    force_format: Option<Language>,
) -> Result<(), IncludeError<Pr::Error>> {
    // model-toml.md "Editing the manifest": on a `model.toml` project
    // `include` appends to `sources` and nothing else -- "Neither
    // extracts symbols or hashes files, as both maps are computed at
    // build".
    //
    // `Both` deliberately falls through to the json arm, where
    // `get_meta` refuses it as the ambiguity it is. Asking `manifest()`
    // instead would have been wrong: it maps `Both` onto "no toml
    // manifest".
    if project.format().map_err(IncludeError::Project)? == ProjectFormat::Toml {
        return include_toml(
            project,
            paths,
            compute_checksum,
            index_symbols,
            force_format,
        );
    }

    // TODO: is `unwrap_or_default()` appropriate here?
    let mut meta = project
        .get_meta()
        .map_err(IncludeError::Project)?
        .unwrap_or_default();
    // The Syside refusal of `--no-index-symbols`, on the extracted form.
    if meta.root.is_some() && !index_symbols {
        return Err(IncludeError::NoIndexSymbolsOnRoot);
    }
    for path in paths {
        let including = "Including";
        let header = crate::style::get_style_config().header;
        log::info!("{header}{including:>12}{header:#} file `{path}`");

        let source = read_project_file_to_string(&project, &path)?;
        if compute_checksum {
            let checksum = sha256_lowercase_hex(&source);
            meta.add_checksum(&path, KerMlChecksumAlg::Sha256, checksum, true);
        } else {
            meta.add_checksum(&path, KerMlChecksumAlg::None, "", true);
        }

        // A root-shaped document has no per-file map to update; its
        // `exports` are recomputed by `build`, and the flag is refused
        // on such a project.
        if index_symbols && let Some(index) = meta.index.as_mut() {
            // Remove if present any existing symbols from the same file
            index.retain(|s, v| {
                if *v == path {
                    log::debug!("meta.index: removing obsolete symbol `{s}` (file `{v}`)");
                    false
                } else {
                    true
                }
            });

            for s in extract_symbols(&path, &source, force_format)? {
                index.insert(s, path.to_string());
            }
        }
    }
    project
        .put_meta(&meta, true)
        .map_err(IncludeError::Project)?;
    Ok(())
}

/// `include` on a `model.toml` project: append each path to `sources`.
///
/// Nothing is read. The file's contents would be hashed and indexed on
/// a json project; here both maps are computed when the KPAR is built,
/// so the only question is whether the path belongs in `sources` --
/// which `Sources::new` answers, as it does on read.
fn include_toml<Pr: ProjectMut, I: Iterator<Item = Utf8UnixPathBuf>>(
    project: &mut Pr,
    paths: I,
    compute_checksum: bool,
    index_symbols: bool,
    force_format: Option<Language>,
) -> Result<(), IncludeError<Pr::Error>> {
    let mut document = project.manifest_document().map_err(manifest_error)?;
    // The file must be of a type the metamodel admits. The metamodel
    // comes from the resolved manifest when the project resolves, and
    // from the document's own literal value otherwise; a member read
    // without its workspace has neither, and the check is skipped rather
    // than the command refusing over something that is not about the
    // path it was given.
    // The next read of the manifest raises the same complaint anyway.
    let metamodel = project_metamodel(project, &document);

    // On a Syside project the flag names an `index` its KPAR does not
    // have, which is a different complaint from "computed at build" --
    // nothing is computed.
    if !index_symbols
        && metamodel
            .as_ref()
            .is_some_and(|kind| kind.family == crate::model::MetamodelFamily::Syside)
    {
        return Err(IncludeError::NoIndexSymbolsOnRoot);
    }
    if compute_checksum {
        return Err(IncludeError::ComputedField("checksum"));
    }
    if !index_symbols {
        return Err(IncludeError::ComputedField("index"));
    }
    if force_format.is_some() {
        return Err(IncludeError::ForceFormat);
    }

    for path in paths {
        let including = "Including";
        let header = crate::style::get_style_config().header;
        log::info!("{header}{including:>12}{header:#} file `{path}`");

        if let Some(kind) = &metamodel
            && let Err(problem) = crate::manifest::common::admitted_source(&path, kind)
        {
            return Err(IncludeError::NotAdmitted(problem.message.into()));
        }

        crate::manifest::edit::include_source(&mut document, path.as_str())
            .map_err(IncludeError::Edit)?;
    }

    project
        .put_manifest_document(&document, true)
        .map_err(|e| match e {
            crate::project::ManifestWriteError::Project(e) => IncludeError::Project(e),
            crate::project::ManifestWriteError::Unsupported => {
                IncludeError::Manifest("this project cannot store a `model.toml`".into())
            }
        })
}

/// Flatten a manifest read failure into this command's error.
///
/// `NoTomlManifest` is unreachable -- the format was just checked --
/// but the match is exhaustive and saying so is better than a wildcard
/// that would swallow a variant added later.
fn manifest_error<E: crate::env::utils::ErrorBound>(
    error: crate::project::ManifestError<E>,
) -> IncludeError<E> {
    match error {
        crate::project::ManifestError::Project(e) => IncludeError::Project(e),
        crate::project::ManifestError::NoTomlManifest => {
            IncludeError::Manifest("this project has no `model.toml` manifest".into())
        }
    }
}

/// Extract top level symbols from `source`, using `path` for diagnostics
/// only.
///
/// A thin wrapper over [`crate::symbols::top_level_symbols`], which is
/// what a caller that is not a command should use: the two failures are
/// the same, and this one only widens them into the error `do_include`
/// returns.
pub fn extract_symbols<P: AsRef<Utf8UnixPath>, S: AsRef<str>, T>(
    path: &P,
    source: S,
    force_format: Option<Language>,
) -> Result<Vec<String>, IncludeError<T>> {
    crate::symbols::top_level_symbols(path, source, force_format).map_err(IncludeError::from)
}

pub fn read_project_file_to_string<Pr: ProjectRead, P: AsRef<Utf8UnixPath>>(
    project: &Pr,
    path: &P,
) -> Result<String, IncludeError<Pr::Error>> {
    let mut file = project.read_source(path).map_err(IncludeError::Project)?;
    let mut source = String::new();
    file.read_to_string(&mut source).map_err(|e| {
        IncludeError::Io(FsIoError::ReadFile(path.as_ref().as_str().into(), e).into())
    })?;
    Ok(source)
}

#[cfg(test)]
#[path = "./include_tests.rs"]
mod tests;

/// The metamodel `include` judges a path against, or `None` when the
/// project cannot say: a member whose `metamodel.workspace = true`
/// cannot be resolved without the workspace table, and the bindings
/// build such a project without one.
fn project_metamodel<Pr: ProjectRead>(
    project: &Pr,
    document: &crate::manifest::edit::DocumentMut,
) -> Option<crate::model::KnownMetamodel> {
    if let Ok(manifest) = project.manifest() {
        return Some(manifest.metamodel);
    }
    let declared = document
        .get("project")?
        .as_table_like()?
        .get("metamodel")?
        .as_str()?;
    let kind = crate::model::KnownMetamodel::recognize(declared);
    if kind.is_none() {
        log::debug!("`{declared}` is not a metamodel this Sysand knows; not checking the language");
    }
    kind
}
