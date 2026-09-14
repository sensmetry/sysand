// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::io::Read as _;

use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::{
    model::KerMlChecksumAlg,
    project::{ProjectMut, ProjectOrIOError, ProjectRead, utils::FsIoError},
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
    // TODO: is `unwrap_or_default()` appropriate here?
    let mut meta = project
        .get_meta()
        .map_err(IncludeError::Project)?
        .unwrap_or_default();
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

        if index_symbols {
            // Remove if present any existing symbols from the same file
            meta.index.retain(|s, v| {
                if *v == path {
                    log::debug!("meta.index: removing obsolete symbol `{s}` (file `{v}`)");
                    false
                } else {
                    true
                }
            });

            for s in extract_symbols(&path, &source, force_format)? {
                meta.index.insert(s, path.to_string());
            }
        }
    }
    project
        .put_meta(&meta, true)
        .map_err(IncludeError::Project)?;
    Ok(())
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
