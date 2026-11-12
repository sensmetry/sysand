// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::io::Read as _;

use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::{
    model::KerMlChecksumAlg,
    project::{
        ProjectDefinition, ProjectFormat, ProjectMut, ProjectOrIOError, ProjectRead,
        utils::FsIoError,
    },
    symbols::{ExtractError, Language, SymbolsError},
    utils::sha256_lowercase_hex,
};

#[derive(Error, Debug)]
pub enum IncludeError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("failed to extract symbol names from `{0}`")]
    Extract(Box<str>, #[source] ExtractError),
    #[error(
        "unknown file format of `{0}`, only SysML v2 (.sysml) and KerML (.kerml) files are supported"
    )]
    UnknownFormat(Box<str>),
    /// The flag controls a map that is computed when the KPAR is built.
    #[error("`{0}` is computed when the KPAR is built and cannot be set on a `model.toml` project")]
    ComputedField(&'static str),
    /// A Syside project's KPAR has no `index` for the flag to apply to.
    #[error(
        "`--no-index-symbols` applies to projects with a computed index; a Syside project's \
         KPAR carries `root` and `exports` instead"
    )]
    NoIndexSymbolsOnRoot,
    /// A `model.toml` project extracts no symbols, so there is no parser to select.
    #[error(
        "`force_format` has no effect on a `model.toml` project, whose symbols are computed when the KPAR is built"
    )]
    ForceFormat,
    /// The file is not of a type the project's metamodel admits.
    #[error("{0}")]
    NotAdmitted(Box<str>),
    #[error("project has no `.project.json` and `.meta.json`")]
    MissingManifest,
    /// The manifest could not be read, or the edit was refused.
    #[error(transparent)]
    Edit(crate::manifest::edit::EditError),
    #[error(transparent)]
    Manifest(crate::project::ManifestError<ProjectError>),
    #[error(transparent)]
    ManifestWrite(crate::project::ManifestWriteError<ProjectError>),
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
    // `Both` falls through to the json arm, which refuses it as ambiguous.
    if project.format().map_err(IncludeError::Project)? == ProjectFormat::Toml {
        return include_toml(
            project,
            paths,
            compute_checksum,
            index_symbols,
            force_format,
        );
    }

    let Some(ProjectDefinition::Json { info: _, mut meta }) =
        project.get_project().map_err(IncludeError::Project)?
    else {
        return Err(IncludeError::MissingManifest);
    };
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

        // A document with `root` has no `index` to update.
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

            for s in crate::symbols::top_level_symbols(&path, &source, force_format)? {
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
/// Symbols and checksums are computed when the KPAR is built, so the path is
/// only added to `sources`.
fn include_toml<Pr: ProjectMut, I: Iterator<Item = Utf8UnixPathBuf>>(
    project: &mut Pr,
    paths: I,
    compute_checksum: bool,
    index_symbols: bool,
    force_format: Option<Language>,
) -> Result<(), IncludeError<Pr::Error>> {
    let mut document = project
        .manifest_document()
        .map_err(IncludeError::Manifest)?;
    // If neither the resolved manifest nor the document gives a metamodel,
    // the file type check is skipped.
    let metamodel = project_metamodel(project, &document);

    // Checked before the "computed at build" refusals, which would be
    // misleading on a Syside project.
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
        .map_err(IncludeError::ManifestWrite)
}

/// The text of one of `project`'s source files.
pub fn read_project_file_to_string<Pr: ProjectRead, P: AsRef<Utf8UnixPath>>(
    project: &Pr,
    path: &P,
) -> Result<String, ProjectOrIOError<Pr::Error>> {
    let mut file = project
        .read_source(path)
        .map_err(ProjectOrIOError::Project)?;
    let mut source = String::new();
    file.read_to_string(&mut source)
        .map_err(|e| FsIoError::ReadFile(path.as_ref().as_str().into(), e))?;
    Ok(source)
}

#[cfg(test)]
#[path = "./include_tests.rs"]
mod tests;

/// The metamodel `include` judges a path against, or `None` when the
/// project cannot say, such as a member with `metamodel.workspace = true`
/// read without its workspace table.
fn project_metamodel<Pr: ProjectRead>(
    project: &Pr,
    document: &crate::manifest::edit::DocumentMut,
) -> Option<crate::model::KnownMetamodel> {
    match project.get_project() {
        Ok(Some(ProjectDefinition::Toml(manifest))) => return Some(manifest.metamodel),
        Ok(_) => {}
        // Falls back to the metamodel the document declares.
        Err(error) => log::debug!("the project's manifest does not resolve: {error}"),
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
