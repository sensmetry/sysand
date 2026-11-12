// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino::{Utf8Path, Utf8PathBuf};
use indexmap::IndexMap;
use jiff::tz::TimeZone;
use thiserror::Error;

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write as _,
};

use crate::{
    env::utils::ErrorBound,
    include::{IncludeError, extract_symbols, read_project_file_to_string},
    manifest::{
        kpar_view::{self, ViewError},
        usage::{Usage, UsageBase},
    },
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsageRaw,
        InterchangeProjectValidationError, KerMlChecksumAlg,
    },
    project::{
        ProjectDefinition, ProjectRead,
        local_kpar::{KparInnerPath, LocalKParError, LocalKParProject, LocalKParProjectRaw},
        local_src::{LocalSrcError, LocalSrcProject},
        utils::{FsIoError, ZipArchiveError, wrapfs},
    },
    utils::{format_err, license_file_stems, sha256_lowercase_hex, to_pretty_json_string},
    workspace::model_toml,
};

#[derive(Default, Copy, Clone, Debug, PartialEq, Eq)]
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
    /// A Syside project's KPAR has no `index` to keep.
    #[error(
        "`--keep-index` applies to projects with a computed index; a Syside project's KPAR \
         carries `root` and `exports` instead"
    )]
    KeepIndexOnRoot,
    #[error(
        "unknown file format of `{0}`, only SysML v2 (.sysml) and KerML (.kerml) files are supported"
    )]
    UnknownFormat(Box<str>),
    /// The path of a `dir` or `kpar-path` usage names nothing that defines a
    /// project, which includes a project with no root at all (an in-memory
    /// project).
    #[error("usage `{usage}` targets `{path}`, which defines no project")]
    TomlTargetMissing { usage: String, path: String },
    /// A project satisfies a usage only if it declares the pair the usage spells.
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
    #[error("usage `{usage}` targets `{path}`, which could not be read")]
    TomlTargetRead {
        usage: String,
        path: String,
        #[source]
        source: Box<TomlTargetReadError>,
    },
    /// A member inheriting a group usage whose path is relative to the
    /// workspace root.
    ///
    /// A group entry's `dir`/`kpar-path` is relative to the directory of
    /// the `model.toml` that defines the workspace, but the build would
    /// resolve it against the member.
    #[error(
        "usage `{usage}` is inherited from the workspace, and its path `{path}` is relative \n\
        {SP:>8} to the workspace root; building a project that inherits a path usage is not \n\
        {SP:>8} supported yet",
        SP = ' '
    )]
    InheritedLocalUsage { usage: String, path: String },
    #[error("missing project info file `.project.json` and metadata file `.meta.json`")]
    MissingInfoMeta,
    #[error("missing project info file `.project.json`")]
    MissingInfo,
    #[error(transparent)]
    Zip(#[from] ZipArchiveError),
    #[error("project serialization error: {0}")]
    Serialize(&'static str, #[source] serde_json::Error),
    #[error(
        "project includes a path usage `{0}`,\n\
        which is unlikely to be available on other computers at the same path"
    )]
    PathUsage(String),
    #[error("file `{0}` is missing symbol `{1}` found in index")]
    MissingIndexSymbol(Box<str>, String),
}

/// Why the target of a local usage could not be read.
#[derive(Error, Debug)]
pub enum TomlTargetReadError {
    #[error(transparent)]
    Kpar(#[from] LocalKParError),
    #[error(transparent)]
    Src(#[from] LocalSrcError),
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
            // Only raised for a `model.toml` project, which a build refuses
            // before extracting symbols.
            IncludeError::ComputedField(_)
            | IncludeError::NoIndexSymbolsOnRoot
            | IncludeError::ForceFormat
            | IncludeError::NotAdmitted(_)
            | IncludeError::MissingManifest
            | IncludeError::Edit(_)
            | IncludeError::Manifest(_)
            | IncludeError::ManifestWrite(_) => Self::Extract(format_err(value)),
        }
    }
}

/// Where a build writes when it is not told otherwise. A workspace has
/// one, at its root; a standalone project has its own.
pub const DEFAULT_OUTPUT_NAME: &str = "output";

/// `workspace_root` is the root of the workspace the project is a member of,
/// if it is one. A workspace has one `output/`, at its root, so a member's
/// archive goes there wherever the build was run from.
pub fn default_kpar_path<Pr: ProjectRead>(
    project: &Pr,
    workspace_root: Option<&Utf8Path>,
    project_path: &Utf8Path,
) -> Result<Utf8PathBuf, KParBuildError<Pr::Error>> {
    Ok(output_path_in(
        workspace_root,
        project_path,
        &default_kpar_file_name(project)?,
    ))
}

fn output_path_in(
    workspace_root: Option<&Utf8Path>,
    project_path: &Utf8Path,
    file_name: &str,
) -> Utf8PathBuf {
    let mut path = workspace_root
        .unwrap_or(project_path)
        .join(DEFAULT_OUTPUT_NAME);
    path.push(file_name);
    path
}

/// [`default_kpar_path`] for a manifest already read.
pub fn default_kpar_path_of(
    definition: &ProjectDefinition,
    workspace_root: Option<&Utf8Path>,
    project_path: &Utf8Path,
) -> Utf8PathBuf {
    output_path_in(workspace_root, project_path, &kpar_file_name_of(definition))
}

pub fn default_kpar_file_name<Pr: ProjectRead>(
    project: &Pr,
) -> Result<String, KParBuildError<Pr::Error>> {
    let Some(definition) = project.get_project().map_err(KParBuildError::ProjectRead)? else {
        return Err(KParBuildError::MissingInfo);
    };
    Ok(kpar_file_name_of(&definition))
}

fn kpar_file_name_of(definition: &ProjectDefinition) -> String {
    format!(
        "{}-{}.kpar",
        definition
            .name()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>(),
        definition.version()
    )
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
    do_build_kpar_with(
        project,
        || project.get_project(),
        path,
        compression,
        update_index,
        allow_path_usage,
    )
}

/// [`do_build_kpar`] for a caller that has the project's manifest in
/// hand: `definition` supplies it, once the build has started.
pub fn do_build_kpar_with<P: AsRef<Utf8Path>, Pr: ProjectRead>(
    project: &Pr,
    definition: impl FnOnce() -> Result<Option<ProjectDefinition>, Pr::Error>,
    path: P,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
) -> Result<LocalKParProjectRaw, KParBuildError<Pr::Error>> {
    let path = path.as_ref();
    match do_build_kpar_inner(
        project,
        definition,
        path,
        compression,
        update_index,
        allow_path_usage,
        None,
    ) {
        Ok(p) => Ok(p),
        Err(e) => {
            if let Err(e) = fs::remove_file(path) {
                log::debug!("cleanup: failed to remove archive file `{path}`: {e}");
            }
            Err(e)
        }
    }
}

/// The created archive is left in place on error; the caller must delete it.
///
/// `definition` reads the project's manifest, once the build has started.
fn do_build_kpar_inner<P: AsRef<Utf8Path>, Pr: ProjectRead>(
    project: &Pr,
    definition: impl FnOnce() -> Result<Option<ProjectDefinition>, Pr::Error>,
    path: P,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
    workspace: TargetWorkspace<'_>,
) -> Result<LocalKParProjectRaw, KParBuildError<Pr::Error>> {
    let building = "Building";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{building:>12}{header:#} kpar `{}`", path.as_ref());

    // A `model.toml` project's documents are written now, recording the
    // build time as `created`. Two builds of identical sources are
    // therefore not byte-identical.
    // A `model.toml` project's view reads, hashes and indexes every source,
    // and the contents it read are kept for the archive.
    let mut viewed: Option<HashMap<String, String>> = None;
    let (mut info, mut meta, manifest) = match definition() {
        Ok(None) => return Err(KParBuildError::MissingInfoMeta),
        Ok(Some(ProjectDefinition::Json { info, meta })) => (info, meta, None),
        Ok(Some(ProjectDefinition::Toml(manifest))) => {
            let mut contents = HashMap::new();
            let (info, meta) = kpar_view::view(&manifest, &jiff::Timestamp::now(), |path| {
                let source = read_project_file_to_string(project, &path)?;
                contents.insert(path.to_string(), source.clone());
                Ok::<_, IncludeError<Pr::Error>>(source)
            })
            .map_err(|error| match error {
                ViewError::Read(error) => KParBuildError::from(error),
                ViewError::Symbols(error) => KParBuildError::from(IncludeError::from(error)),
            })?;
            viewed = Some(contents);
            (info, meta, Some(manifest))
        }
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

    // Archive rules the KPAR view leaves out. Only for a `model.toml`
    // project: a json `.project.json` may legally carry a
    // `pkg:sysand/omg/...` resource, and a json project with a typed path
    // usage builds under `--allow-path-usage`.
    if let Some(manifest) = manifest {
        // The typed usages know each usage's kind and target.
        let root = project.project_root();
        for (located, raw) in manifest.usages.iter().zip(info.usage.iter_mut()) {
            // An inherited usage's paths are relative to the workspace root,
            // which is not known here.
            if located.base != UsageBase::Project
                && let Some(path) = located.usage.local_path()
            {
                return Err(KParBuildError::InheritedLocalUsage {
                    usage: located.usage.to_string(),
                    path: path.to_string(),
                });
            }
            match &located.usage {
                Usage::SourcePath {
                    dir,
                    publisher: _,
                    name: _,
                } if !allow_path_usage => {
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

        // A project carrying `stdlib` has no publisher, so a locally built
        // standard-library KPAR matches the published one.
        if manifest.stdlib.is_some() {
            info.publisher = None;
        }
    }

    if let Some(path_usage) = info.usage.iter().find_map(|x| {
        match x {
            InterchangeProjectUsageRaw::Resource {
                resource,
                version_constraint: _,
            } => is_file_url(resource).then_some(resource),
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
            // Portable: resolved by identity or an `http(s)` URL wherever the
            // KPAR is used
            InterchangeProjectUsageRaw::RemoteKpar {
                kpar_url: _,
                publisher: _,
                name: _,
            }
            | InterchangeProjectUsageRaw::Index {
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

    // Sorted, so identical builds write the same `.meta.json`.
    let mut source_paths: Vec<_> = meta.source_paths(true).into_iter().collect();
    source_paths.sort();
    let mut checksums = match meta.checksum.take() {
        Some(checksum) if viewed.is_some() => checksum,
        Some(mut checksum) => {
            checksum.clear();
            checksum
        }
        None => IndexMap::new(),
    };
    let len = source_paths.len();
    // A root-shaped document has `exports` in place of `index`, so
    // `--keep-index` has nothing to keep.
    let root_shape = meta.root.is_some();
    if root_shape && !update_index {
        return Err(KParBuildError::KeepIndexOnRoot);
    }
    if let Some(mut contents) = viewed {
        for (i, p) in source_paths.into_iter().enumerate() {
            eprint!("\rwriting files ({}/{len})", i + 1);

            let source = match contents.remove(&p) {
                Some(source) => source,
                None => read_project_file_to_string(project, &p)?,
            };
            write_entry(&mut zip, &p, &source, options, path.as_ref())?;
        }
    } else {
        let mut index = IndexMap::new();
        let mut exports: Vec<String> = Vec::new();
        let progress = if update_index {
            "updating file metadata"
        } else {
            "updating file checksums"
        };
        for (i, p) in source_paths.into_iter().enumerate() {
            // `log` always appends a newline, so `\r` does not work with it
            eprint!("\r{progress} ({}/{len})", i + 1);

            let source = read_project_file_to_string(project, &p)?;
            let checksum = sha256_lowercase_hex(&source);
            let symbols = extract_symbols(&p, &source, None)?;
            if update_index {
                for s in symbols {
                    if root_shape {
                        // Each name once; sorted below.
                        if !exports.contains(&s) {
                            exports.push(s);
                        }
                    } else {
                        index.insert(s, p.clone());
                    }
                }
            } else {
                check_kept_index(&meta, &p, symbols)?;
            }

            write_entry(&mut zip, &p, &source, options, path.as_ref())?;
            checksums.insert(
                p,
                InterchangeProjectChecksumRaw {
                    value: checksum,
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                },
            );
        }
        if update_index {
            if root_shape {
                // Each name once, sorted.
                exports.sort();
                meta.exports = Some(exports);
            } else {
                meta.index = Some(index);
            }
        }
    }
    eprintln!();

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
    // For the root shape `checksum` covers every file in the archive but the
    // two json manifests. An OMG archive checksums its sources only.
    if root_shape {
        for (archive_path, content) in &extra_files {
            checksums.insert(
                archive_path.clone(),
                InterchangeProjectChecksumRaw {
                    value: sha256_lowercase_hex(content),
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                },
            );
        }
    }
    meta.checksum = Some(checksums);

    for (archive_path, content) in extra_files {
        write_entry(&mut zip, &archive_path, &content, options, path.as_ref())?;
    }

    // KerML Clause 10.3: “In addition, the archive shall contain, at its
    // top level, exactly one file named .project.json and exactly one file
    // named .meta.json.”

    let info_content = to_pretty_json_string(&info);
    let meta_content = to_pretty_json_string(&meta);

    write_entry(
        &mut zip,
        ".project.json",
        &info_content,
        options,
        path.as_ref(),
    )?;
    write_entry(
        &mut zip,
        ".meta.json",
        &meta_content,
        options,
        path.as_ref(),
    )?;

    zip.finish()
        .map_err(|e| ZipArchiveError::Finish(path.as_ref().into(), e))?;

    Ok(LocalKParProjectRaw::new_project_at_root(&path)?)
}

/// Write one file into the archive at `archive`.
fn write_entry<E: ErrorBound>(
    zip: &mut zip::ZipWriter<std::fs::File>,
    name: &str,
    content: &str,
    options: zip::write::SimpleFileOptions,
    archive: &Utf8Path,
) -> Result<(), KParBuildError<E>> {
    zip.start_file(name, options)
        .map_err(|e| ZipArchiveError::Write(Utf8Path::new(name).into(), e))?;
    zip.write_all(content.as_bytes())
        .map_err(|e| FsIoError::WriteFile(archive.into(), e))?;
    Ok(())
}

/// Check a file's symbols against the index kept from its `.meta.json`:
/// a symbol the index lists but the file lacks is an error, and one the
/// index lacks is a warning.
fn check_kept_index<E: ErrorBound>(
    meta: &InterchangeProjectMetadataRaw,
    path: &str,
    symbols: Vec<String>,
) -> Result<(), KParBuildError<E>> {
    let new_symbols: HashSet<String> = symbols.into_iter().collect();
    let old_symbols = meta.file_index_symbols(path);
    if let Some(only_in_old) = old_symbols.difference(&new_symbols).next() {
        return Err(KParBuildError::MissingIndexSymbol(
            path.into(),
            only_in_old.clone(),
        ));
    }
    for only_in_new in new_symbols.difference(&old_symbols) {
        // TODO: figure out a way to only print suggestions when running the CLI
        log::warn!(
            "index is missing symbol `{only_in_new}` found in file `{path}`;\n\
            if this is not intentional, include the file again to update its\n\
            exported symbols, or omit `--keep-index` to do so for all files"
        );
    }
    Ok(())
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

/// The workspace a build is part of, for reading a `dir` target.
///
/// A member that inherits a key resolves its manifest only against the
/// table, so a target read without one fails with "not a member of any
/// workspace", although it is one.
type TargetWorkspace<'a> =
    Option<&'a std::sync::Arc<crate::manifest::workspace_table::WorkspaceTable>>;

/// A local usage becomes a typed index usage constrained to `^<version>`,
/// `<version>` being the target's version read from its manifest at build
/// time. Building never resolves usages, so it needs neither network nor index.
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
    // A `model.toml` usage is satisfied by a target that declares the pair
    // as the usage spells it. A build reads its targets itself, so this is
    // checked here as well as in the resolver.
    let spelled = |declared_publisher: Option<&str>, declared_name: &str| {
        // A built standard-library archive declares no publisher. The name
        // is still compared; whether a publisher-less target may be used at
        // all is for the resolver to decide.
        let publisher_agrees = match (declared_publisher, usage.publisher()) {
            (Some(declared), Some(spelled)) => declared == spelled.as_str(),
            // Either side declaring none compares on the name alone.
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
    let unreadable = |source: TomlTargetReadError| KParBuildError::TomlTargetRead {
        usage: usage.to_string(),
        path: relative.to_owned(),
        source: Box::new(source),
    };

    let Some(root) = root else {
        // Nothing to join onto.
        return Err(missing());
    };
    let target = root.join(relative);

    // Stat `target` before joining anything onto it: for a file (every kpar
    // target, or a `dir` entry naming a file) that gives `ENOTDIR`, an IO
    // error where `TomlTargetMissing` belongs.
    match std::fs::metadata(&target) {
        Ok(metadata) if metadata.is_dir() != is_kpar => {}
        Ok(_) | Err(_) => return Err(missing()),
    }

    let version = if is_kpar {
        // `Root` is what the resolver uses for a `kpar-path` usage.
        let kpar = LocalKParProject::new_access(target, KparInnerPath::Root, None);
        match kpar.get_project() {
            Ok(Some(definition)) => {
                spelled(definition.publisher(), definition.name())?;
                definition.version()
            }
            Ok(None) => return Err(missing()),
            Err(error) => return Err(unreadable(error.into())),
        }
    } else {
        let source = match workspace {
            Some(table) => LocalSrcProject::new_member(target, None, table.clone()),
            None => LocalSrcProject::new_access(target, None),
        };
        match source.get_project() {
            Ok(Some(ProjectDefinition::Toml(manifest))) => {
                spelled(Some(manifest.publisher.as_str()), manifest.name.as_str())?;
                // A usage whose target carries `stdlib` is written as
                // `resource = "<target's metamodel>/<target's stdlib>.kpar"`
                // rather than as a typed index usage. The constraint is
                // the same `^<version>` as for any other local target.
                if let Some(stem) = &manifest.stdlib {
                    // `resolve` refuses `stdlib` unless the metamodel is a
                    // recognized OMG one, so the metamodel is a valid prefix.
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
            // A json target, the one `TomlTargetVersion` is for.
            Ok(Some(ProjectDefinition::Json { info, meta: _ })) => {
                spelled(info.publisher.as_deref(), &info.name)?;
                info.version
            }
            Ok(None) => return Err(missing()),
            Err(error) => return Err(unreadable(error.into())),
        }
    };

    let Some(publisher) = usage.publisher() else {
        unreachable!("a local usage names its publisher");
    };
    Ok(InterchangeProjectUsageRaw::Index {
        publisher: publisher.as_str().to_owned(),
        name: usage.name().as_str().to_owned(),
        version_constraint: caret(&version, usage, relative)?,
    })
}

/// `^<version>`, or [`KParBuildError::TomlTargetVersion`].
fn caret<E: ErrorBound>(
    version: &str,
    usage: &Usage,
    relative: &str,
) -> Result<String, KParBuildError<E>> {
    semver::VersionReq::parse(&format!("^{version}"))
        .map(|constraint| constraint.to_string())
        .map_err(|_parse_error| KParBuildError::TomlTargetVersion {
            usage: usage.to_string(),
            path: relative.to_owned(),
            version: version.to_owned(),
        })
}

/// Every member, with the manifest the workspace load parsed.
fn read_members(workspace: &model_toml::Workspace) -> Vec<Member> {
    // Every member carries the workspace table: a member that inherits a
    // key resolves its manifest only with the table in hand.
    let table = workspace.table().clone();
    workspace
        .members()
        .iter()
        .map(|member| {
            let dir = canonical_or(member.dir.clone());
            let project = LocalSrcProject::new_member(dir.clone(), None, table.clone());
            let definition = ProjectDefinition::Toml(Box::new(member.project.clone()));
            Member {
                dir,
                project,
                definition,
            }
        })
        .collect()
}

/// The members in dependency order, each with the archive `sysand build`
/// writes for it, `<root>/output/<name>-<version>.kpar`.
///
pub fn workspace_publish_order(
    workspace: &model_toml::Workspace,
) -> Vec<(Utf8PathBuf, Utf8PathBuf)> {
    let root = workspace.root();
    workspace
        .members_in_dependency_order()
        .into_iter()
        .map(|member| {
            let dir = canonical_or(member.dir.clone());
            let file_name =
                kpar_file_name_of(&ProjectDefinition::Toml(Box::new(member.project.clone())));
            let path = output_path_in(Some(root), &dir, &file_name);
            (shorten(&dir, root), path)
        })
        .collect()
}

/// Build every member, in `members` order, and return the archives.
///
/// A member that fails does not stop the others. The error is every member
/// that failed, together with the archives the others produced.
pub fn do_build_workspace_kpars<P: AsRef<Utf8Path>>(
    workspace: &model_toml::Workspace,
    path: P,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
) -> Result<Vec<LocalKParProjectRaw>, WorkspaceBuildFailure> {
    let root = workspace.root();
    // A `dir` usage of a member that inherits keys needs the table to read the target.
    let table = workspace.table().clone();
    let mut built = Vec::new();
    let mut failed = Vec::new();
    for member in read_members(workspace) {
        let dir = member.dir.clone();
        match build_member(
            member,
            path.as_ref(),
            compression,
            update_index,
            allow_path_usage,
            Some(&table),
        ) {
            Ok(kpar) => built.push(kpar),
            Err(error) => failed.push(MemberFailure::at(&shorten(&dir, root), error)),
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
    /// to the workspace root, as the `members` entries name it.
    pub failed: Vec<MemberFailure>,
    /// The archives of the members that built, in `members` order.
    pub built: Vec<LocalKParProjectRaw>,
}

/// A member, named relative to the workspace root, and what went wrong
/// with it.
///
/// Boxed because `KParBuildError` is large.
#[derive(Debug, Error)]
#[error("workspace member `{member}` cannot be built")]
pub struct MemberFailure {
    pub member: Utf8PathBuf,
    #[source]
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
fn build_member(
    member: Member,
    output: &Utf8Path,
    compression: KparCompressionMethod,
    update_index: bool,
    allow_path_usage: bool,
    workspace: TargetWorkspace<'_>,
) -> Result<LocalKParProjectRaw, KParBuildError<LocalSrcError>> {
    let Member {
        dir: _,
        project,
        definition,
    } = member;
    let output_path = output.join(kpar_file_name_of(&definition));
    do_build_kpar_inner(
        &project,
        || Ok(Some(definition)),
        &output_path,
        compression,
        update_index,
        allow_path_usage,
        workspace,
    )
    .inspect_err(|_| {
        if let Err(error) = fs::remove_file(&output_path) {
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
    definition: ProjectDefinition,
}

/// `path`, canonicalized, or `path` itself where it cannot be.
///
/// A member whose directory does not exist is still a member: it gets a
/// node, it gets built, and it fails with its own message.
fn canonical_or(path: Utf8PathBuf) -> Utf8PathBuf {
    wrapfs::canonicalize(&path).unwrap_or(path)
}

/// Whether `url` has the `file:` scheme, matched case-insensitively.
fn is_file_url(url: &str) -> bool {
    url.get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
}

/// A member directory as the workspace names it.
fn shorten(dir: &Utf8Path, root: &Utf8Path) -> Utf8PathBuf {
    dir.strip_prefix(root).unwrap_or(dir).to_path_buf()
}

#[cfg(test)]
#[path = "./build_tests.rs"]
mod tests;
