// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino::{Utf8Path, Utf8PathBuf};
use indexmap::IndexMap;
use thiserror::Error;

use std::{collections::HashSet, io::Write as _};

use crate::{
    env::utils::ErrorBound,
    include::{IncludeError, extract_symbols, read_project_file_to_string},
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectUsageRaw,
        InterchangeProjectValidationError, KerMlChecksumAlg,
    },
    project::{
        ProjectRead,
        local_kpar::LocalKParProjectRaw,
        local_src::{LocalSrcError, LocalSrcProject},
        utils::{FsIoError, ZipArchiveError, wrapfs},
    },
    utils::{license_file_stems, sha256_lowercase_hex},
    workspace::{Workspace, WorkspaceReadError},
};

#[derive(Default, Copy, Clone, Debug, PartialEq, Eq)]
// Currently python interop is done with strings instead
// to have less boilerplate
// #[cfg_attr(feature = "python", pyclass(eq))]
pub enum KparCompressionMethod {
    /// Store the files as is
    Stored,
    /// Compress the files using Deflate
    #[default]
    Deflated,
    /// Compress the files using BZIP2
    #[cfg(feature = "kpar-bzip2")]
    Bzip2,
    /// Compress the files using ZStandard
    #[cfg(feature = "kpar-zstd")]
    Zstd,
    /// Compress the files using XZ
    #[cfg(feature = "kpar-xz")]
    Xz,
    /// Compress the files using PPMd
    #[cfg(feature = "kpar-ppmd")]
    Ppmd,
}

impl From<KparCompressionMethod> for zip::CompressionMethod {
    fn from(value: KparCompressionMethod) -> Self {
        match value {
            KparCompressionMethod::Stored => zip::CompressionMethod::Stored,
            KparCompressionMethod::Deflated => zip::CompressionMethod::Deflated,
            #[cfg(feature = "kpar-bzip2")]
            KparCompressionMethod::Bzip2 => zip::CompressionMethod::Bzip2,
            #[cfg(feature = "kpar-zstd")]
            KparCompressionMethod::Zstd => zip::CompressionMethod::Zstd,
            #[cfg(feature = "kpar-xz")]
            KparCompressionMethod::Xz => zip::CompressionMethod::Xz,
            #[cfg(feature = "kpar-ppmd")]
            KparCompressionMethod::Ppmd => zip::CompressionMethod::Ppmd,
        }
    }
}

#[derive(Debug, Error)]
pub enum CompressionMethodParseError {
    #[error("Compile sysand with feature {feature} to use {compression} compression")]
    SuggestFeature {
        compression: String,
        feature: String,
    },
    #[error("{0}")]
    Invalid(String),
}

impl TryFrom<String> for KparCompressionMethod {
    type Error = CompressionMethodParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl TryFrom<&str> for KparCompressionMethod {
    type Error = CompressionMethodParseError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "STORED" => Ok(KparCompressionMethod::Stored),
            "DEFLATED" => Ok(KparCompressionMethod::Deflated),
            #[cfg(feature = "kpar-bzip2")]
            "BZIP2" => Ok(KparCompressionMethod::Bzip2),
            #[cfg(not(feature = "kpar-bzip2"))]
            "BZIP2" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-bzip2".into(),
            }),
            #[cfg(feature = "kpar-zstd")]
            "ZSTD" => Ok(KparCompressionMethod::Zstd),
            #[cfg(not(feature = "kpar-zstd"))]
            "ZSTD" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-zstd".into(),
            }),
            #[cfg(feature = "kpar-xz")]
            "XZ" => Ok(KparCompressionMethod::Xz),
            #[cfg(not(feature = "kpar-xz"))]
            "XZ" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-xz".into(),
            }),
            #[cfg(feature = "kpar-ppmd")]
            "PPMD" => Ok(KparCompressionMethod::Ppmd),
            #[cfg(not(feature = "kpar-ppmd"))]
            "PPMD" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-ppmd".into(),
            }),
            _ => Err(CompressionMethodParseError::Invalid(format!(
                "Compression method `{value}` is invalid"
            ))),
        }
    }
}

#[derive(Error, Debug)]
pub enum KParBuildError<ProjectReadError: ErrorBound> {
    #[error(transparent)]
    ProjectRead(ProjectReadError),
    #[error(transparent)]
    WorkspaceRead(#[from] WorkspaceReadError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("project's `.{name}.json` is invalid")]
    Validation {
        name: &'static str,
        source: InterchangeProjectValidationError,
    },
    #[error("{0}")]
    Extract(String),
    #[error(
        "unknown file format of `{0}`, only SysML v2 (.sysml) and KerML (.kerml) files are supported"
    )]
    UnknownFormat(Box<str>),
    #[error("missing project info file `.project.json` and metadata file `.meta.json`")]
    MissingInfoMeta,
    #[error("missing project info file `.project.json`")]
    MissingInfo,
    #[error("missing project metadata file `.meta.json`")]
    MissingMeta,
    #[error(transparent)]
    Zip(#[from] ZipArchiveError),
    #[error("project serialization error: {0}: {1}")]
    Serialize(&'static str, serde_json::Error),
    #[error(
        "project includes a path usage `{0}`,\n\
        which is unlikely to be available on other computers at the same path"
    )]
    PathUsage(String),
    #[error(
        "workspace sets metamodel `{workspace_metamodel}`, but project `{project_path}` \
         sets a different metamodel `{project_metamodel}` in `.meta.json`;\n\
         remove the metamodel from the project's `.meta.json` or from `.workspace.json`"
    )]
    WorkspaceMetamodelConflict {
        workspace_metamodel: String,
        project_metamodel: String,
        project_path: String,
    },
    #[error("file `{0}` is missing symbol `{1}` found in index")]
    MissingIndexSymbol(Box<str>, String),
}

impl<ProjectReadError: ErrorBound> From<FsIoError> for KParBuildError<ProjectReadError> {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl<ProjectReadError: ErrorBound> From<IncludeError<ProjectReadError>>
    for KParBuildError<ProjectReadError>
{
    fn from(value: IncludeError<ProjectReadError>) -> Self {
        match value {
            IncludeError::Project(error) => Self::ProjectRead(error),
            IncludeError::Io(error) => error.into(),
            IncludeError::Extract(..) => Self::Extract(value.to_string()),
            IncludeError::UnknownFormat(error) => Self::UnknownFormat(error),
        }
    }
}

pub fn default_kpar_path<Pr: ProjectRead>(
    project: &Pr,
    workspace: Option<&Workspace>,
    project_path: &Utf8Path,
) -> Result<Utf8PathBuf, KParBuildError<Pr::Error>> {
    let mut path = workspace
        .map(Workspace::root_path)
        .unwrap_or(project_path)
        .join("output");
    path.push(default_kpar_file_name(project)?);
    Ok(path)
}

pub fn default_kpar_file_name<Pr: ProjectRead>(
    project: &Pr,
) -> Result<String, KParBuildError<Pr::Error>> {
    let Some(project_info) = project.get_info().map_err(KParBuildError::ProjectRead)? else {
        return Err(KParBuildError::MissingInfo);
    };
    Ok(format!(
        "{}-{}.kpar",
        project_info
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>(),
        project_info.version
    ))
}

/// `update_index` controls whether to parse symbols from current
/// file to update index
pub fn do_build_kpar<P: AsRef<Utf8Path>, Pr: ProjectRead>(
    project: &Pr,
    path: P,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
) -> Result<LocalKParProjectRaw, KParBuildError<Pr::Error>> {
    let path = path.as_ref();
    match do_build_kpar_inner(
        project,
        path,
        compression,
        update_index,
        allow_path_usage,
        None,
    ) {
        Ok(p) => Ok(p),
        Err(e) => {
            if let Err(e) = wrapfs::remove_file(path) {
                log::debug!("cleanup: failed to remove archive file `{path}`: {e}");
            }
            Err(e)
        }
    }
}

/// Caller must delete the created archive on error
fn do_build_kpar_inner<P: AsRef<Utf8Path>, Pr: ProjectRead>(
    project: &Pr,
    path: P,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
    workspace_metamodel: Option<&str>,
) -> Result<LocalKParProjectRaw, KParBuildError<Pr::Error>> {
    let building = "Building";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{building:>12}{header:#} kpar `{}`", path.as_ref());

    let (info, mut meta) = match project.get_project() {
        Ok(im) => match im {
            (Some(i), Some(m)) => (i, m),
            (None, Some(_)) => return Err(KParBuildError::MissingInfo),
            (Some(_), None) => return Err(KParBuildError::MissingMeta),
            (None, None) => return Err(KParBuildError::MissingInfoMeta),
        },
        Err(e) => return Err(KParBuildError::ProjectRead(e)),
    };
    meta.validate().map_err(|e| KParBuildError::Validation {
        name: "meta",
        source: e,
    })?;

    match semver::Version::parse(&info.version) {
        Ok(_) => (),
        Err(e) => log::warn!(
            "project's version `{}` is not a valid SemVer version: {e}",
            info.version
        ),
    }
    let license_info: Option<(&str, spdx::Expression)> =
        info.license
            .as_deref()
            .and_then(|l| match spdx::Expression::parse(l) {
                Ok(expr) => Some((l, expr)),
                Err(e) => {
                    log::warn!(
                        "project's license `{l}` is not a valid SPDX license expression:\n{e}"
                    );
                    None
                }
            });

    if let Some(resource) = info.usage.iter().find_map(|x| {
        // Case-insensitively match `file:` scheme
        match x {
            InterchangeProjectUsageRaw::Resource { resource, .. } => {
                if let Some(scheme) = resource.get(..5)
                    && scheme.eq_ignore_ascii_case("file:")
                {
                    Some(resource)
                } else {
                    None
                }
            }
        }
    }) {
        if allow_path_usage {
            log::warn!(
                "project includes a path usage `{resource}`,\n\
                which is unlikely to be available on other computers at the same path"
            );
        } else {
            return Err(KParBuildError::PathUsage(resource.clone()));
        }
    }

    if let Some(ws_metamodel) = workspace_metamodel {
        if let Some(proj_metamodel) = &meta.metamodel {
            if proj_metamodel != ws_metamodel {
                return Err(KParBuildError::WorkspaceMetamodelConflict {
                    workspace_metamodel: ws_metamodel.to_string(),
                    project_metamodel: proj_metamodel.into(),
                    project_path: path.as_ref().to_string(),
                });
            }
        } else {
            meta.metamodel = Some(ws_metamodel.to_string());
        }
    }

    let archive_file = wrapfs::File::create(&path)?;
    let mut zip = zip::ZipWriter::new(archive_file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(compression.into())
        .system(zip::System::Unix)
        .last_modified_time(zip::DateTime::DEFAULT);

    let source_paths = meta.source_paths(true);
    let mut checksums = if let Some(mut checksum) = meta.checksum.take() {
        checksum.clear();
        checksum
    } else {
        IndexMap::new()
    };
    let len = source_paths.len();
    if update_index {
        meta.index.clear();
        for (i, p) in source_paths.into_iter().enumerate() {
            // `log` always appends a newline, so `\r` does not work with it
            eprint!("\rupdating file metadata ({}/{len})", i + 1);

            let source = read_project_file_to_string(project, &p)?;
            let checksum = sha256_lowercase_hex(&source);
            let symbols = extract_symbols(&p, &source, None)?;

            zip.start_file(&p, options)
                .map_err(|e| ZipArchiveError::Write(Utf8Path::new(&p).into(), e))?;
            zip.write_all(source.as_bytes())
                .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;

            for s in symbols {
                meta.index.insert(s, p.clone());
            }
            checksums.insert(
                p,
                InterchangeProjectChecksumRaw {
                    value: checksum,
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                },
            );
        }
    } else {
        for (i, p) in source_paths.into_iter().enumerate() {
            eprint!("\rupdating file checksums ({}/{len})", i + 1);

            let source = read_project_file_to_string(project, &p)?;
            let checksum = sha256_lowercase_hex(&source);
            let new_symbols = extract_symbols(&p, &source, None)?;

            let new_symbols: HashSet<String> = new_symbols.into_iter().collect();
            let old_symbols = meta.file_index_symbols(&p);
            if let Some(only_in_old) = old_symbols.difference(&new_symbols).next() {
                return Err(KParBuildError::MissingIndexSymbol(
                    p.as_str().into(),
                    only_in_old.clone(),
                ));
            }
            for only_in_new in new_symbols.difference(&old_symbols) {
                // TODO: figure out a way to only print suggestions when running the CLI
                log::warn!(
                    "index is missing symbol `{only_in_new}` found in file `{p}`;\n\
                    if this is not intentional, include the file again to update its\n\
                    exported symbols, or omit `--keep-index` to do so for all files"
                );
            }

            zip.start_file(&p, options)
                .map_err(|e| ZipArchiveError::Write(Utf8Path::new(&p).into(), e))?;
            zip.write_all(source.as_bytes())
                .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;

            checksums.insert(
                p,
                InterchangeProjectChecksumRaw {
                    value: checksum,
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                },
            );
        }
    }
    eprintln!();
    meta.checksum = Some(checksums);

    let project_root = project.project_root();
    let mut extra_files: Vec<(String, String)> = Vec::new();

    if let Some(content) = read_optional_project_file(project_root, "README.md", "readme")? {
        extra_files.push(("README.md".to_string(), content));
    }
    if let Some(content) = read_optional_project_file(project_root, "CHANGELOG.md", "changelog")? {
        extra_files.push(("CHANGELOG.md".to_string(), content));
    }
    if let Some((license_str, expression)) = license_info.as_ref() {
        for stem in license_file_stems(expression) {
            let relative = format!("LICENSES/{stem}.txt");
            match read_optional_project_file(project_root, &relative, "license")? {
                Some(content) => extra_files.push((relative, content)),
                None => log::warn!(
                    "license file `{relative}` referenced by project license `{license_str}` was not found"
                ),
            }
        }
    }
    for (archive_path, content) in extra_files {
        zip.start_file(&archive_path, options)
            .map_err(|e| ZipArchiveError::Write(Utf8Path::new(&archive_path).into(), e))?;
        zip.write_all(content.as_bytes())
            .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;
    }

    // KerML Clause 10.3: “In addition, the archive shall contain, at its
    // top level, exactly one file named .project.json and exactly one file
    // named .meta.json.”

    let info_content =
        serde_json::to_string(&info).expect("BUG: failed to serialize .project.json");
    let meta_content = serde_json::to_string(&meta).expect("BUG: failed to serialize .meta.json");

    zip.start_file(".project.json", options)
        .map_err(|e| ZipArchiveError::Write(Utf8Path::new(".project.json").into(), e))?;
    zip.write_all(info_content.as_bytes())
        .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;

    zip.start_file(".meta.json", options)
        .map_err(|e| ZipArchiveError::Write(Utf8Path::new(".meta.json").into(), e))?;
    zip.write_all(meta_content.as_bytes())
        .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;

    zip.finish()
        .map_err(|e| ZipArchiveError::Finish(path.as_ref().into(), e))?;

    Ok(LocalKParProjectRaw::new_project_at_root(&path)?)
}

fn read_optional_project_file(
    project_root: Option<&Utf8Path>,
    file_name: &str,
    log_label: &str,
) -> Result<Option<String>, FsIoError> {
    let Some(file_path) = project_root.map(|p| p.join(file_name)) else {
        return Ok(None);
    };
    match std::fs::read_to_string(&file_path) {
        Ok(content) => {
            let header = crate::style::get_style_config().header;
            let including = "Including";
            log::info!("{header}{including:>12}{header:#} {log_label} from `{file_path}`");
            Ok(Some(content))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(FsIoError::ReadFile(file_path, e)),
    }
}

pub fn do_build_workspace_kpars<P: AsRef<Utf8Path>>(
    workspace: &Workspace,
    path: P,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
) -> Result<Vec<LocalKParProjectRaw>, KParBuildError<LocalSrcError>> {
    let ws_metamodel = workspace.metamodel().map(|iri| iri.as_str());

    let mut result = Vec::new();
    for project_root in workspace.projects() {
        let project = LocalSrcProject {
            nominal_path: None,
            project_path: workspace.root_path().join(&project_root.path),
            expected_checksum: None,
        };

        let file_name = default_kpar_file_name(&project)?;
        let output_path = path.as_ref().join(file_name);
        let kpar_project = match do_build_kpar_inner(
            &project,
            &output_path,
            compression,
            update_index,
            allow_path_usage,
            ws_metamodel,
        ) {
            Ok(p) => p,
            Err(e) => {
                if let Err(e) = wrapfs::remove_file(&output_path) {
                    log::debug!("cleanup: failed to remove archive file `{output_path}`: {e}");
                }
                return Err(e);
            }
        };
        result.push(kpar_project);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "./build_tests.rs"]
mod tests;
