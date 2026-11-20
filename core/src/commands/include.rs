// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::io::Read as _;

use thiserror::Error;
use typed_path::Utf8UnixPath;
#[cfg(feature = "filesystem")]
use typed_path::Utf8UnixPathBuf;

#[cfg(feature = "filesystem")]
use crate::{
    context::ManifestEdit,
    manifest::{
        common::{self, PathKey},
        edit::{self, DocumentMut},
        project::ResolvedProject,
    },
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, KerMlChecksumAlg},
    project::local_src::{LocalSrcError, LocalSrcProject},
    symbols::Language,
    utils::sha256_lowercase_hex,
};
use crate::{
    manifest::edit::EditError,
    project::{ProjectOrIOError, ProjectRead, utils::FsIoError},
    symbols::{ExtractError, SymbolsError},
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
    /// A Syside-metamodel `model.toml` has no `sources` to append to.
    #[error("a Syside-metamodel project lists no `sources`; its files are reached from its `root`")]
    RootOnly,
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
    Edit(EditError),
    #[error(transparent)]
    Manifest(#[from] crate::project::ManifestError<ProjectError>),
    #[error(transparent)]
    ManifestWrite(#[from] crate::project::ManifestWriteError<ProjectError>),
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

/// Including source files in the current project.
///
/// A json project gets a checksum and, when it has an `index`, the symbols
/// of each file. A `model.toml` project gets the path appended to `sources`:
/// its symbols and checksums are computed when the KPAR is built.
// TODO: Add a CLI option to make the file format explicit (useful in cases
// of non-standard file extensions)
#[cfg(feature = "filesystem")]
#[derive(Debug)]
pub struct IncludeFiles {
    pub paths: Vec<Utf8UnixPathBuf>,
    pub compute_checksum: bool,
    pub index_symbols: bool,
    pub force_format: Option<Language>,
}

#[cfg(feature = "filesystem")]
impl ManifestEdit for IncludeFiles {
    type Output = ();
    type Error = IncludeError<LocalSrcError>;

    fn toml(
        self,
        document: &mut DocumentMut,
        project: &ResolvedProject,
    ) -> Result<(), Self::Error> {
        include_toml(document, project, self)
    }

    fn json(
        self,
        project: &LocalSrcProject,
        _info: &mut InterchangeProjectInfoRaw,
        meta: &mut InterchangeProjectMetadataRaw,
    ) -> Result<(), Self::Error> {
        include_json(project, meta, self)
    }
}

/// `include` on a json project.
#[cfg(feature = "filesystem")]
fn include_json<Pr: ProjectRead>(
    project: &Pr,
    meta: &mut InterchangeProjectMetadataRaw,
    include: IncludeFiles,
) -> Result<(), IncludeError<Pr::Error>> {
    let IncludeFiles {
        paths,
        compute_checksum,
        index_symbols,
        force_format,
    } = include;
    if meta.root.is_some() && !index_symbols {
        return Err(IncludeError::NoIndexSymbolsOnRoot);
    }
    for path in paths {
        let including = "Including";
        let header = crate::style::get_style_config().header;
        log::info!("{header}{including:>12}{header:#} file `{path}`");

        let source = read_project_file_to_string(project, &path)?;
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
    Ok(())
}

/// `include` on a `model.toml` project: append each path to `sources`.
///
/// Symbols and checksums are computed when the KPAR is built, so the path is
/// only added to `sources`.
#[cfg(feature = "filesystem")]
fn include_toml<E>(
    document: &mut DocumentMut,
    project: &ResolvedProject,
    include: IncludeFiles,
) -> Result<(), IncludeError<E>> {
    let IncludeFiles {
        paths,
        compute_checksum,
        index_symbols,
        force_format,
    } = include;
    let metamodel = &project.metamodel;

    // Checked before the "computed at build" refusals, which would be
    // misleading on a Syside project.
    if metamodel.family == crate::model::MetamodelFamily::Syside {
        return Err(IncludeError::RootOnly);
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

        if let Err(problem) = common::admitted_source(PathKey::Sources, &path, metamodel) {
            return Err(IncludeError::NotAdmitted(problem.message.into()));
        }

        edit::include_source(document, path.as_str()).map_err(IncludeError::Edit)?;
    }
    Ok(())
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
