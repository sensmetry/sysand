// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino::{Utf8Path, Utf8PathBuf};
use indexmap::IndexMap;
use jiff::tz::TimeZone;
use thiserror::Error;

use std::{collections::HashSet, io::Write as _};

use crate::{
    env::utils::ErrorBound,
    graph::DependencyGraph,
    include::{IncludeError, extract_symbols, read_project_file_to_string},
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
    workspace::model_toml,
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
    /// model-toml.md "Normalization", at build time: a project satisfies
    /// a usage only if it declares the pair the usage spells. A build
    /// reads its targets directly rather than through a resolver, so
    /// without this a usage could name one project and be built against
    /// another.
    #[error(
        "usage `{usage}` targets `{path}`, which declares `{declared}`;\n\
        {SP:>8} a usage spells publisher and name as the target declares them",
        SP = ' '
    )]
    TomlTargetSpelling {
        usage: String,
        path: String,
        declared: String,
    },
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
            | IncludeError::NotAdmitted(_)
            | IncludeError::Edit(_)
            | IncludeError::Manifest(_) => Self::Extract(format_err(value)),
        }
    }
}

/// workspaces.md "Build output": where a build writes when it is not
/// told otherwise. A workspace has one, at its root; a standalone
/// project has its own.
pub const DEFAULT_OUTPUT_NAME: &str = "output";

/// `workspace_root` is the root of the workspace the project is a
/// member of, if it is one: workspaces.md "Build output" gives a
/// workspace **one** `output/`, at its root, so a member's archive goes
/// there whether the build was run from the root or from the member.
/// The root alone is what that needs -- a loaded [`model_toml::Workspace`]
/// would mean reading the members to answer a question about a path.
pub fn default_kpar_path<Pr: ProjectRead>(
    project: &Pr,
    workspace_root: Option<&Utf8Path>,
    project_path: &Utf8Path,
) -> Result<Utf8PathBuf, KParBuildError<Pr::Error>> {
    let mut path = workspace_root
        .unwrap_or(project_path)
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
    workspace: TargetWorkspace<'_>,
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
        // remote-kpar `kpar-url` that happens to be a `pkg:sysand` IRI would
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
                Usage::SourcePath { dir, .. } if !allow_path_usage => {
                    *raw =
                        local_target_usage(root, &located.usage, dir.as_str(), false, workspace)?;
                }
                Usage::KparPath {
                    kpar_path,
                    publisher: _,
                    name: _,
                } if !allow_path_usage => {
                    *raw = local_target_usage(
                        root,
                        &located.usage,
                        kpar_path.as_str(),
                        true,
                        workspace,
                    )?;
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
/// The workspace a build is part of, for reading a `dir` target.
///
/// A member that inherits a key resolves its manifest only against the
/// table, so a target read without one fails with "not a member of any
/// workspace" -- about a project that is one.
type TargetWorkspace<'a> =
    Option<&'a std::sync::Arc<crate::manifest::workspace_table::WorkspaceTable>>;

fn local_target_usage<E: ErrorBound>(
    root: Option<&Utf8Path>,
    usage: &Usage,
    relative: &str,
    is_kpar: bool,
    workspace: TargetWorkspace<'_>,
) -> Result<InterchangeProjectUsageRaw, KParBuildError<E>> {
    let missing = || KParBuildError::TomlTargetMissing {
        usage: usage.to_string(),
        path: relative.to_owned(),
    };
    // model-toml.md "Normalization": a `model.toml` usage is satisfied
    // by a target that declares the pair as the usage spells it.
    // Checked here as well as in the resolver, because a build reads
    // its targets itself and never asks one.
    let spelled = |declared_publisher: Option<&str>, declared_name: &str| {
        // A built standard-library archive is the ordinary example of a
        // target that declares no publisher at all -- the build drops
        // the publisher on purpose. The name is still compared; whether
        // a publisher-less target may be used at all is the resolver's
        // question.
        let publisher_agrees = match (declared_publisher, usage.publisher()) {
            (Some(declared), Some(spelled)) => declared == spelled.as_str(),
            // Either side declaring none compares on the name alone: a
            // publisher-less target, or a standard-library usage.
            _ => true,
        };
        if publisher_agrees && declared_name == usage.name().as_str() {
            Ok(())
        } else {
            Err(KParBuildError::TomlTargetSpelling {
                usage: usage.to_string(),
                path: relative.to_owned(),
                declared: crate::project::utils::pair(declared_publisher, declared_name),
            })
        }
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
        match kpar.get_project() {
            Ok((Some(info), _)) => {
                spelled(info.publisher.as_deref(), &info.name)?;
                info.version
            }
            // An archive with no `.project.json` defines no project.
            Ok((None, _)) => return Err(missing()),
            Err(error) => return Err(unreadable(&error)),
        }
    } else {
        let source = match workspace {
            Some(table) => LocalSrcProject::new_member(target, None, table.clone()),
            None => LocalSrcProject::new_access(target, None),
        };
        // `manifest()` and not `version()`: the latter goes through
        // `get_info`, which for a toml target builds the whole KPAR
        // view -- reading, hashing and symbol-extracting every file the
        // target's `sources` lists. A target with one missing source
        // would fail *this* project's build.
        match source.manifest() {
            Ok(manifest) => {
                spelled(Some(manifest.publisher.as_str()), manifest.name.as_str())?;
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
                    // A resolved project declares one of the four
                    // metamodels, and `resolve` refuses `stdlib` unless
                    // it is a recognized OMG one, so the prefix is
                    // there to be written.
                    return Ok(InterchangeProjectUsageRaw::Resource {
                        resource: format!("{}/{stem}.kpar", manifest.metamodel),
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
            Err(ManifestError::NoTomlManifest) => match source.get_project() {
                Ok((Some(info), _)) => {
                    spelled(info.publisher.as_deref(), &info.name)?;
                    info.version
                }
                Ok((None, _)) => return Err(missing()),
                Err(error) => return Err(unreadable(&error)),
            },
            Err(ManifestError::Project(error)) => return Err(unreadable(&error)),
        }
    };

    Ok(InterchangeProjectUsageRaw::Resource {
        resource: usage.identity().lock_identifier(),
        version_constraint: Some(caret(&version, usage, relative)?),
    })
}

/// `^<version>`, or [`KParBuildError::TomlTargetVersion`].
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

/// Every member, read once: a `LocalSrcProject` caches its own view, and
/// two instances of the same member would read it twice.
fn read_members(workspace: &model_toml::Workspace) -> Vec<Member> {
    // Every member carries the workspace table: a member that inherits a
    // key resolves its manifest only with the table in hand.
    let table = std::sync::Arc::new(workspace.table().clone());
    workspace
        .members()
        .iter()
        .map(|member| {
            let dir = canonical_or(member.dir.clone());
            let project = LocalSrcProject::new_member(dir.clone(), None, table.clone());
            Member { dir, project }
        })
        .collect()
}

/// The members in dependency order, each with the archive `sysand build`
/// writes for it -- `<root>/output/<name>-<version>.kpar`.
///
/// The archive name needs the member's own name and version, so every
/// member is read here, before the caller uploads anything: publishing
/// half a workspace because the last member was unreadable is worse than
/// publishing none.
///
/// The error names the member, because `KParBuildError` does not: a
/// failure report whose entries cannot be told apart is a count, not a
/// report.
pub fn workspace_publish_order(
    workspace: &model_toml::Workspace,
) -> Result<Vec<(Utf8PathBuf, Utf8PathBuf)>, MemberFailure> {
    let root = workspace.root();
    let members = read_members(workspace);

    // workspaces.md "Build output": members are published in dependency
    // order, a topological sort of their `dir` usages of each other.
    //
    // Every member is a node, whether or not its directory resolves: a
    // member that cannot be read must fail with its own message, not
    // vanish from the order.
    let mut graph = DependencyGraph::new();
    for member in &members {
        graph.add_node(member.dir.clone());
    }
    for member in &members {
        for dependency in member_dependencies(&member.project) {
            // `add_edge` ignores an endpoint the graph does not hold,
            // which is what a usage of a project outside the workspace
            // is.
            graph.add_edge(&member.dir, &canonical_or(member.dir.join(dependency)));
        }
    }

    let mut archives = Vec::new();
    for dir in graph.dependency_order() {
        for member in members.iter().filter(|member| &member.dir == dir) {
            let short = shorten(&member.dir, root);
            let archive = default_kpar_path(&member.project, Some(root), &member.dir)
                .map_err(|error| MemberFailure::at(&short, error))?;
            archives.push((short, archive));
        }
    }
    Ok(archives)
}

/// Build every member, in `members` order, and return the archives.
///
/// workspaces.md "Build output": a build reads a target's `version` from
/// its manifest and never from a built artifact, so no member's build
/// needs another's, and a member that fails does not stop the others.
/// The error is every member that failed, together with the archives the
/// others produced.
pub fn do_build_workspace_kpars<P: AsRef<Utf8Path>>(
    workspace: &model_toml::Workspace,
    path: P,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
) -> Result<Vec<LocalKParProjectRaw>, WorkspaceBuildFailure> {
    let root = workspace.root();
    // The same table the members were read with: a `dir` usage of a
    // member that inherits a key needs it to read the target.
    let table = std::sync::Arc::new(workspace.table().clone());
    let mut built = Vec::new();
    let mut failed = Vec::new();
    for member in read_members(workspace) {
        match build_member(
            &member,
            path.as_ref(),
            compression,
            update_index,
            allow_path_usage,
            Some(&table),
        ) {
            Ok(kpar) => built.push(kpar),
            Err(error) => failed.push(MemberFailure::at(&shorten(&member.dir, root), error)),
        }
    }
    if failed.is_empty() {
        Ok(built)
    } else {
        Err(WorkspaceBuildFailure { failed, built })
    }
}

/// A workspace build in which at least one member failed.
#[derive(Debug, Error)]
#[error(
    "{} of {} workspace members failed to build",
    failed.len(),
    failed.len() + built.len()
)]
pub struct WorkspaceBuildFailure {
    /// Every member that failed, in `members` order, each named relative
    /// to the workspace root -- the way the `members` entries name it.
    pub failed: Vec<MemberFailure>,
    /// The archives of the members that built, in `members` order.
    pub built: Vec<LocalKParProjectRaw>,
}

/// A member, named relative to the workspace root, and what went wrong
/// with it.
///
/// A named type rather than a tuple because it is an error, and boxed
/// because `KParBuildError` is large enough that returning it by value
/// in a `Result` is its own lint.
#[derive(Debug, Error)]
#[error("`{member}`: {error}")]
pub struct MemberFailure {
    pub member: Utf8PathBuf,
    pub error: Box<KParBuildError<LocalSrcError>>,
}

impl MemberFailure {
    fn at(member: &Utf8Path, error: KParBuildError<LocalSrcError>) -> Self {
        Self {
            member: member.to_owned(),
            error: Box::new(error),
        }
    }
}

/// One member's build, with its archive removed if it failed.
///
/// `default_kpar_file_name` is inside this rather than outside it: for a
/// member with no manifest that call *is* the failure, and propagating
/// it would let the first unreadable member stop the workspace.
fn build_member(
    member: &Member,
    output: &Utf8Path,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
    workspace: TargetWorkspace<'_>,
) -> Result<LocalKParProjectRaw, KParBuildError<LocalSrcError>> {
    let output_path = output.join(default_kpar_file_name(&member.project)?);
    do_build_kpar_inner(
        &member.project,
        &output_path,
        compression,
        update_index,
        allow_path_usage,
        workspace,
    )
    .inspect_err(|_| {
        if let Err(error) = wrapfs::remove_file(&output_path) {
            log::debug!("cleanup: failed to remove archive file `{output_path}`: {error}");
        }
    })
}

/// A workspace member, read once.
struct Member {
    /// The node this member is in the dependency graph: its directory,
    /// canonical where the filesystem allows it.
    dir: Utf8PathBuf,
    project: LocalSrcProject,
}

/// `path`, canonicalized, or `path` itself where it cannot be.
///
/// A member whose directory does not exist is still a member: it gets a
/// node, it gets built, and it fails with its own message.
fn canonical_or(path: Utf8PathBuf) -> Utf8PathBuf {
    wrapfs::canonicalize(&path).unwrap_or(path)
}

/// The `dir` usages a member declares, as written: what the publish
/// order needs from it.
///
/// For a toml member the reader is `manifest()`, not `usage()`: the
/// latter goes through `get_info`, which builds the whole KPAR view --
/// reading, hashing and symbol-extracting every source file the member
/// lists. A json member has no `model.toml`, so it answers
/// `NoTomlManifest` and its usages come from `.project.json`.
///
/// A member that cannot be read at all contributes no edges. It fails
/// with a message about what is actually wrong with it as soon as its
/// archive name is asked for.
fn member_dependencies(project: &LocalSrcProject) -> Vec<String> {
    match project.manifest() {
        Ok(manifest) => manifest
            .usages
            .iter()
            .filter_map(|located| match &located.usage {
                Usage::SourcePath {
                    dir,
                    publisher: _,
                    name: _,
                } => Some(dir.as_str().to_owned()),
                _ => None,
            })
            .collect(),
        Err(ManifestError::NoTomlManifest) => match project.get_info() {
            Ok(Some(info)) => info
                .usage
                .iter()
                .filter_map(|usage| match usage {
                    InterchangeProjectUsageRaw::Directory {
                        dir,
                        publisher: _,
                        name: _,
                    } => Some(dir.as_str().to_owned()),
                    _ => None,
                })
                .collect(),
            Ok(None) | Err(_) => Vec::new(),
        },
        Err(ManifestError::Project(_)) => Vec::new(),
    }
}

/// A member directory as the workspace names it.
fn shorten(dir: &Utf8Path, root: &Utf8Path) -> Utf8PathBuf {
    dir.strip_prefix(root).unwrap_or(dir).to_path_buf()
}

#[cfg(test)]
#[path = "./build_tests.rs"]
mod tests;
