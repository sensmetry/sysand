// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino::{Utf8Path, Utf8PathBuf};
use indexmap::IndexMap;
use jiff::tz::TimeZone;
use thiserror::Error;

use std::{collections::HashSet, io::Write as _};

use crate::{
    env::utils::ErrorBound,
    include::{IncludeError, extract_symbols, read_project_file_to_string},
    manifest::common::normalized_identity,
    manifest::usage::{Usage, UsageBase},
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectUsageRaw,
        InterchangeProjectValidationError, KerMlChecksumAlg, format_created,
    },
    project::{
        ManifestError, ProjectFormat, ProjectRead,
        local_kpar::{KparInnerPath, LocalKParProject, LocalKParProjectRaw},
        local_src::{LocalSrcError, LocalSrcProject},
        utils::{FsIoError, ZipArchiveError, wrapfs},
    },
    utils::{format_err, license_file_stems, sha256_lowercase_hex, to_pretty_json_string},
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
            KparCompressionMethod::Stored => Self::Stored,
            KparCompressionMethod::Deflated => Self::Deflated,
            #[cfg(feature = "kpar-bzip2")]
            KparCompressionMethod::Bzip2 => Self::Bzip2,
            #[cfg(feature = "kpar-zstd")]
            KparCompressionMethod::Zstd => Self::Zstd,
            #[cfg(feature = "kpar-xz")]
            KparCompressionMethod::Xz => Self::Xz,
            #[cfg(feature = "kpar-ppmd")]
            KparCompressionMethod::Ppmd => Self::Ppmd,
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
            "STORED" => Ok(Self::Stored),
            "DEFLATED" => Ok(Self::Deflated),
            #[cfg(feature = "kpar-bzip2")]
            "BZIP2" => Ok(Self::Bzip2),
            #[cfg(not(feature = "kpar-bzip2"))]
            "BZIP2" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-bzip2".into(),
            }),
            #[cfg(feature = "kpar-zstd")]
            "ZSTD" => Ok(Self::Zstd),
            #[cfg(not(feature = "kpar-zstd"))]
            "ZSTD" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-zstd".into(),
            }),
            #[cfg(feature = "kpar-xz")]
            "XZ" => Ok(Self::Xz),
            #[cfg(not(feature = "kpar-xz"))]
            "XZ" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-xz".into(),
            }),
            #[cfg(feature = "kpar-ppmd")]
            "PPMD" => Ok(Self::Ppmd),
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
    /// model-toml.md "TOML to KPAR": a `dir` or `kpar-path` usage
    /// "turn[s] into a PURL constrained to `^<version>`, where
    /// `<version>` is the target project's version at build time".
    ///
    /// The path names nothing that defines a project. This also covers
    /// a project with no root at all -- an in-memory project in the
    /// bindings -- whose targets cannot be read, since from the user's
    /// side those are the same sentence.
    #[error("usage `{usage}` targets `{path}`, which defines no project")]
    TomlTargetMissing { usage: String, path: String },
    /// Reachable only for a json target: a `model.toml` constrains its
    /// own `version` to semver, so a toml target has one by
    /// construction.
    #[error(
        "usage `{usage}` targets `{path}`, whose version `{version}` is not semver,\n\
        {SP:>8} so no `^{version}` constraint can be written;\n\
        {SP:>8} pass `--allow-path-usage` to keep the usage as a path usage instead",
        SP = ' '
    )]
    TomlTargetVersion {
        usage: String,
        path: String,
        version: String,
    },
    /// The target is there and does not read: it holds both manifests,
    /// its `model.toml` does not parse, its `.project.json` has no
    /// `version`, its archive is not a zip.
    ///
    /// Its own variant because [`KParBuildError::ProjectRead`] carries
    /// `Pr::Error` -- the error type of the project being *built* --
    /// and a target is read with a different reader.
    #[error("usage `{usage}` targets `{path}`, which could not be read: {reason}")]
    TomlTargetRead {
        usage: String,
        path: String,
        reason: String,
    },
    /// model-toml.md "TOML to KPAR": "an index, source path or kpar path
    /// usage whose target carries `stdlib`: the OMG IRI instead of the
    /// PURL". For an index usage the target is not on disk at all -- the
    /// IRI comes from the embedded metadata of `known_std_libs()` -- so
    /// this one is WP-6b-ii.
    ///
    /// `--allow-path-usage` does not excuse it: the flag is about path
    /// usages, and "under `--allow-path-usage` a local usage stays a
    /// typed path usage, stdlib target or not" says nothing about an
    /// index usage.
    #[error(
        "building a `model.toml` project with the standard-library usage `{usage}` is not \n\
        {SP:>8} supported yet; this version of Sysand cannot rewrite it into the OMG IRI\n\
        {SP:>8} an archive carries",
        SP = ' '
    )]
    TomlStdlibUsage { usage: String },
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
            IncludeError::Extract(..) => Self::Extract(format_err(value)),
            IncludeError::UnknownFormat(error) => Self::UnknownFormat(error),
            // These belong to `do_include`'s `model.toml` arm, and a
            // build refuses such a project before it gets here. The arms
            // exist because the match is exhaustive, not because they
            // are reachable.
            IncludeError::ComputedField(_)
            | IncludeError::ForceFormat
            | IncludeError::Edit(_)
            | IncludeError::Manifest(_) => Self::Extract(format_err(value)),
        }
    }
}

/// workspaces.md "Build output": where a build writes when it is not
/// told otherwise. A workspace has one, at its root; a standalone
/// project has its own.
pub const DEFAULT_OUTPUT_NAME: &str = "output";

pub fn default_kpar_path<Pr: ProjectRead>(
    project: &Pr,
    workspace: Option<&Workspace>,
    project_path: &Utf8Path,
) -> Result<Utf8PathBuf, KParBuildError<Pr::Error>> {
    let mut path = workspace
        .map_or(project_path, Workspace::root_path)
        .join(DEFAULT_OUTPUT_NAME);
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

    let (mut info, mut meta) = match project.get_project() {
        Ok(im) => match im {
            (Some(i), Some(m)) => (i, m),
            (None, Some(_)) => return Err(KParBuildError::MissingInfo),
            (Some(_), None) => return Err(KParBuildError::MissingMeta),
            (None, None) => return Err(KParBuildError::MissingInfoMeta),
        },
        Err(e) => return Err(KParBuildError::ProjectRead(e)),
    };
    let info_valid = info.validate().map_err(|e| KParBuildError::Validation {
        name: "project",
        source: e,
    })?;
    meta.validate().map_err(|e| KParBuildError::Validation {
        name: "meta",
        source: e,
    })?;

    // model-toml.md "TOML to KPAR", the three rules that are the
    // archive's and not the view's (see `kpar_view`'s module doc), and
    // the `created` the view deliberately does not record.
    //
    // Gated on the format: a json `.project.json` may legally carry a
    // `pkg:sysand/omg/...` resource, and a json project with a typed
    // path usage builds under `--allow-path-usage`. Neither is
    // this branch's business.
    if project.format().map_err(KParBuildError::ProjectRead)? == ProjectFormat::Toml {
        let manifest = project.manifest().map_err(|error| match error {
            ManifestError::Project(error) => KParBuildError::ProjectRead(error),
            // `get_project` above produced a view, which only a
            // `model.toml` project can do.
            ManifestError::NoTomlManifest => unreachable!("a toml project has a manifest"),
        })?;

        // Over the *typed* usages, not `info.usage`: at the raw level an
        // index usage and a remote-kpar usage are both `Resource`, and a
        // remote-kpar `url` that happens to be a `pkg:sysand` IRI would
        // be misread as the first.
        //
        // `zip` and not an index range: `kpar_view::info` builds the raw
        // list as `project.usages.iter().map(usage).collect()`, 1:1 and
        // unfiltered, so the two are the same usages in the same order
        // -- and a zip cannot go out of step where a counter can.
        let root = project.project_root();
        for (located, raw) in manifest.usages.iter().zip(info.usage.iter_mut()) {
            // An inherited usage's paths are relative to the workspace
            // root, not the project's, and the arms below resolve
            // against the project root: a workspace-based usage reaching
            // them would silently resolve against the wrong base.
            debug_assert!(
                matches!(located.base, UsageBase::Project),
                "a workspace-based usage needs `resolve_local_path`"
            );
            match &located.usage {
                Usage::Index { publisher, .. } if publisher.is_stdlib_publisher() => {
                    return Err(KParBuildError::TomlStdlibUsage {
                        usage: located.usage.to_string(),
                    });
                }
                Usage::SourcePath { dir, .. } if !allow_path_usage => {
                    *raw = local_target_usage(root, &located.usage, dir.as_str(), false)?;
                }
                Usage::KparPath { kpar_path, .. } if !allow_path_usage => {
                    *raw = local_target_usage(root, &located.usage, kpar_path.as_str(), true)?;
                }
                _ => {}
            }
        }

        // "`created` is the time of the build (so two builds of
        // identical sources are not byte-identical)". The view records
        // the epoch, because a view is a function of the manifest and
        // the source files and the clock is not one of its inputs.
        meta.created = format_created(&jiff::Timestamp::now());

        // "`publisher` goes into `project.publisher`, unless the project
        // carries `stdlib`", so that a locally built standard-library
        // KPAR matches the published one.
        if manifest.stdlib.is_some() {
            info.publisher = None;
        }
    }

    if let Some(path_usage) = info.usage.iter().find_map(|x| {
        match x {
            InterchangeProjectUsageRaw::Resource {
                resource,
                version_constraint: _,
            } => {
                // Case-insensitively match `file:` scheme
                if let Some(scheme) = resource.get(..5)
                    && scheme.eq_ignore_ascii_case("file:")
                {
                    Some(resource)
                } else {
                    None
                }
            }
            InterchangeProjectUsageRaw::Directory {
                dir,
                publisher: _,
                name: _,
            } => Some(dir),
            InterchangeProjectUsageRaw::KparPath {
                kpar_path,
                publisher: _,
                name: _,
            } => Some(kpar_path),
            // Portable: resolved by identity wherever the KPAR is used
            InterchangeProjectUsageRaw::Index {
                publisher: _,
                name: _,
                version_constraint: _,
            } => None,
        }
    }) {
        if allow_path_usage {
            log::warn!(
                "project includes a path usage of `{path_usage}`,\n\
                which is unlikely to be available on other computers at the same path"
            );
        } else {
            return Err(KParBuildError::PathUsage(path_usage.clone()));
        }
    }

    if let Some(ws_metamodel) = workspace_metamodel {
        if let Some(proj_metamodel) = &meta.metamodel {
            if proj_metamodel != ws_metamodel {
                return Err(KParBuildError::WorkspaceMetamodelConflict {
                    workspace_metamodel: ws_metamodel.to_owned(),
                    project_metamodel: proj_metamodel.into(),
                    project_path: path.as_ref().to_string(),
                });
            }
        } else {
            meta.metamodel = Some(ws_metamodel.to_owned());
        }
    }

    let archive_file = wrapfs::File::create(&path)?;
    let mut zip = zip::ZipWriter::new(archive_file);

    let time = jiff::Timestamp::now().to_zoned(TimeZone::UTC).datetime();
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(compression.into())
        .system(zip::System::Unix)
        .last_modified_time(zip::DateTime::try_from(time).unwrap());

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
        extra_files.push(("README.md".to_owned(), content));
    }
    if let Some(content) = read_optional_project_file(project_root, "CHANGELOG.md", "changelog")? {
        extra_files.push(("CHANGELOG.md".to_owned(), content));
    }
    if let Some(expression) = &info_valid.license {
        for stem in license_file_stems(expression) {
            let relative = format!("LICENSES/{stem}.txt");
            match read_optional_project_file(project_root, &relative, "license")? {
                Some(content) => extra_files.push((relative, content)),
                None => log::warn!(
                    "license file `{relative}` referenced by project license `{expression}` was not found"
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

    let info_content = to_pretty_json_string(&info);
    let meta_content = to_pretty_json_string(&meta);

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

/// model-toml.md "TOML to KPAR": a local usage "turn[s] into a PURL
/// constrained to `^<version>`, where `<version>` is the target
/// project's version at build time".
///
/// "`build` never resolves usages, so building needs neither network nor
/// index; the target versions it needs come from the targets' manifests
/// (members and local paths), never from built artifacts."
fn local_target_usage<E: ErrorBound>(
    root: Option<&Utf8Path>,
    usage: &Usage,
    relative: &str,
    is_kpar: bool,
) -> Result<InterchangeProjectUsageRaw, KParBuildError<E>> {
    let missing = || KParBuildError::TomlTargetMissing {
        usage: usage.to_string(),
        path: relative.to_owned(),
    };
    let unreadable = |error: &dyn std::fmt::Display| KParBuildError::TomlTargetRead {
        usage: usage.to_string(),
        path: relative.to_owned(),
        reason: error.to_string(),
    };

    let Some(root) = root else {
        // Nothing to join onto, so the path names nothing this build
        // can see.
        return Err(missing());
    };
    let target = root.join(relative);

    // `std::fs::metadata` before anything that joins onto `target`: for
    // a path that is itself a file -- every kpar target, and a `dir`
    // entry naming a file -- joining and stat-ing gives `ENOTDIR`, which
    // is an IO error where `TomlTargetMissing` belongs.
    match std::fs::metadata(&target) {
        Ok(metadata) if metadata.is_dir() != is_kpar => {}
        Ok(_) | Err(_) => return Err(missing()),
    }

    let version = if is_kpar {
        // `Root` is what the resolver uses for a `kpar-path` usage.
        let kpar = LocalKParProject::new_access(target, KparInnerPath::Root, None);
        match kpar.version() {
            Ok(Some(version)) => version,
            // An archive with no `.project.json` defines no project.
            Ok(None) => return Err(missing()),
            Err(error) => return Err(unreadable(&error)),
        }
    } else {
        let source = LocalSrcProject::new_access(target, None);
        // `manifest()` and not `version()`: the latter goes through
        // `get_info`, which for a toml target builds the whole KPAR
        // view -- reading, hashing and symbol-extracting every file the
        // target's `sources` lists. A target with one missing source
        // would fail *this* project's build.
        match source.manifest() {
            Ok(manifest) => {
                // model-toml.md "Standard library": "a usage whose
                // target carries `stdlib` is written as `resource =
                // \"<target's metamodel>/<target's stdlib>.kpar\"` --
                // the metamodel supplies the prefix and release date --
                // with its version constraint, instead of as a PURL".
                //
                // Only the resource changes: "instead of the PURL"
                // names one substitution, so the constraint stays the
                // `^<version>` every other local target gets.
                if let Some(stem) = &manifest.stdlib {
                    let Some(metamodel) = &manifest.metamodel else {
                        // `ProjectManifest::resolve` refuses `stdlib`
                        // unless the metamodel is a recognized OMG one,
                        // so this cannot happen -- and a `format!` over
                        // an absent prefix would write
                        // `/Semantic-Library.kpar`.
                        return Err(unreadable(&format_args!(
                            "it carries `stdlib` but declares no `metamodel`, \
                             so no standard-library IRI can be written"
                        )));
                    };
                    return Ok(InterchangeProjectUsageRaw::Resource {
                        resource: format!("{metamodel}/{stem}.kpar"),
                        version_constraint: Some(caret(
                            &manifest.version.to_string(),
                            usage,
                            relative,
                        )?),
                    });
                }
                // A `model.toml` constrains its own version to semver,
                // so `TomlTargetVersion` cannot arise here.
                manifest.version.to_string()
            }
            // A json target, the one `TomlTargetVersion` is for:
            // "possible for a json target, since `model.toml` constrains
            // only its own version".
            Err(ManifestError::NoTomlManifest) => match source.version() {
                Ok(Some(version)) => version,
                Ok(None) => return Err(missing()),
                Err(error) => return Err(unreadable(&error)),
            },
            Err(ManifestError::Project(error)) => return Err(unreadable(&error)),
        }
    };

    Ok(InterchangeProjectUsageRaw::Resource {
        resource: normalized_identity(usage.publisher(), usage.name()).lock_identifier(),
        version_constraint: Some(caret(&version, usage, relative)?),
    })
}

/// `^<version>`, or appendix A row 40.
fn caret<E: ErrorBound>(
    version: &str,
    usage: &Usage,
    relative: &str,
) -> Result<String, KParBuildError<E>> {
    // The parse error is discarded deliberately: the error's message is
    // about there being no `^<v>` to write, and semver's own complaint
    // about the string would say less than the version does.
    semver::VersionReq::parse(&format!("^{version}"))
        .map(|constraint| constraint.to_string())
        .map_err(|_parse_error| KParBuildError::TomlTargetVersion {
            usage: usage.to_string(),
            path: relative.to_owned(),
            version: version.to_owned(),
        })
}

pub fn do_build_workspace_kpars<P: AsRef<Utf8Path>>(
    workspace: &Workspace,
    path: P,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
) -> Result<Vec<LocalKParProjectRaw>, KParBuildError<LocalSrcError>> {
    let ws_metamodel = workspace.metamodel().map(ToString::to_string);

    let mut result = Vec::new();
    for project_root in workspace.projects() {
        let project =
            LocalSrcProject::new_access(workspace.root_path().join(&project_root.path), None);
        let file_name = default_kpar_file_name(&project)?;
        let output_path = path.as_ref().join(file_name);
        let kpar_project = match do_build_kpar_inner(
            &project,
            &output_path,
            compression,
            update_index,
            allow_path_usage,
            ws_metamodel.as_deref(),
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
