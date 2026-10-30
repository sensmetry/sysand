// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The KPAR view of a `model.toml` project: the `.project.json` and
//! `.meta.json` pair a reader is handed when it asks such a project for
//! its manifest.
//!
//! A *build* writes the same pair into an archive, and the view follows
//! it wherever it can. Three rules are the archive's alone, and each is
//! marked "the archive" below:
//!
//! 1. `publisher` is omitted from a `stdlib` project's `.project.json`;
//! 2. `dir` and `kpar-path` usages become typed index usages constrained
//!    to `^<target version>`;
//! 3. a usage whose target carries `stdlib` becomes an OMG IRI.
//!
//! All three need the project's targets, so all three belong to the build.
//! What is left is a function of one manifest, free of Cargo features so
//! that bindings without a filesystem can produce it too.

use indexmap::IndexMap;
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::manifest::common::Sources;
use crate::manifest::diagnostics::Report;
use crate::manifest::project::{ProjectManifest, ResolvedProject};
use crate::manifest::usage::Usage;
use crate::manifest::workspace_table::WorkspaceTable;
use crate::manifest::{MODEL_TOML_FILE, ModelToml};
use crate::model::{
    IndexName, IndexPublisher, InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw,
    InterchangeProjectMetadataRaw, InterchangeProjectUsageRaw, KerMlChecksumAlg, format_created,
};
use crate::project::utils::{ProjectDeserializationError, validate_manifest};
use crate::symbols::{SymbolsError, top_level_symbols};
use crate::utils::sha256_lowercase_hex;

/// `index`, as a `.meta.json` carries it: every top level symbol mapped
/// onto the source file that declares it.
pub type SymbolIndex = IndexMap<String, String>;

/// `checksum`, as a `.meta.json` carries it: one entry per source file.
pub type ChecksumMap = IndexMap<String, InterchangeProjectChecksumRaw>;

/// What a view records as `created`.
///
/// A build records the instant it ran. A view must not: the project
/// checksum covers the whole `.meta.json`, is written into the lockfile
/// and is recomputed and compared by `sysand sync`, so a view is a
/// function of the manifest and the source files only.
pub const VIEW_CREATED: jiff::Timestamp = jiff::Timestamp::UNIX_EPOCH;

/// Why a view could not be built.
#[derive(Debug, Error)]
pub enum ViewError<E> {
    /// The `model.toml` is not one, or does not define a project, or
    /// does not validate.
    #[error(transparent)]
    Manifest(#[from] ProjectDeserializationError),
    /// The caller could not hand over a source file's contents. Passed
    /// through: the caller's error already names the file.
    #[error(transparent)]
    Read(E),
    #[error(transparent)]
    Symbols(#[from] SymbolsError),
}

/// Why a json usage has no `model.toml` form.
///
/// Two refusals: a resource usage has no manifest kind, and an
/// identifier is fixed by editing the target's `.project.json`.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NoManifestForm {
    /// A resource usage: a manifest has no such kind.
    #[error(
        "a `model.toml` project cannot declare a resource usage (`{0}`):\n\
        {SP:>8} use an index usage, `dir` or `kpar-path`",
        SP = ' '
    )]
    Resource(String),
    /// The target declares a publisher or name a manifest cannot hold.
    #[error(
        "`{file}` declares {problem};\n\
        {SP:>8} a `model.toml` usage cannot name it",
        SP = ' '
    )]
    Identifier { file: String, problem: String },
}

/// Read a json usage as a manifest usage.
///
/// The reverse of [`usage`], but not its inverse: the forward map turns a
/// `kpar-url` usage into a bare IRI, dropping the publisher and name a
/// manifest needs, so nothing can produce a `kpar-url` entry from json.
///
/// `file` names the document the raw usage came from, for the one
/// refusal whose fix is in another project's manifest.
pub fn usage_of_raw(raw: &InterchangeProjectUsageRaw, file: &str) -> Result<Usage, NoManifestForm> {
    let identity =
        |publisher: &str, name: &str| -> Result<(IndexPublisher, IndexName), NoManifestForm> {
            let publisher =
                IndexPublisher::parse(publisher).map_err(|e| NoManifestForm::Identifier {
                    file: file.to_owned(),
                    problem: format!("`{}`: {e}", e.value),
                })?;
            let name = IndexName::parse(name).map_err(|e| NoManifestForm::Identifier {
                file: file.to_owned(),
                problem: format!("`{}`: {e}", e.value),
            })?;
            Ok((publisher, name))
        };

    match raw {
        InterchangeProjectUsageRaw::Resource {
            resource,
            version_constraint: _,
        } => Err(NoManifestForm::Resource(resource.clone())),
        InterchangeProjectUsageRaw::Directory {
            dir,
            publisher,
            name,
        } => {
            let (publisher, name) = identity(publisher, name)?;
            Ok(Usage::SourcePath {
                publisher,
                name,
                dir: dir.as_str().into(),
            })
        }
        InterchangeProjectUsageRaw::KparPath {
            kpar_path,
            publisher,
            name,
        } => {
            let (publisher, name) = identity(publisher, name)?;
            Ok(Usage::KparPath {
                publisher,
                name,
                kpar_path: kpar_path.as_str().into(),
            })
        }
        // A typed index usage already carries everything a manifest
        // entry needs
        InterchangeProjectUsageRaw::Index {
            publisher,
            name,
            version_constraint,
        } => {
            let (publisher, name) = identity(publisher, name)?;
            let version_constraint =
                semver::VersionReq::parse(version_constraint).map_err(|e| {
                    NoManifestForm::Identifier {
                        file: file.to_owned(),
                        problem: format!("version constraint `{version_constraint}`: {e}"),
                    }
                })?;
            Ok(Usage::Index {
                publisher,
                name,
                version_constraint,
            })
        }
    }
}

/// One usage as a `.project.json` carries it.
///
/// An index usage becomes a typed index usage with the constraint as
/// declared, and a remote-kpar usage its URL as-is. The two local kinds
/// become typed *path* usages here, which is where the view and the
/// archive part company: the archive rewrites them into typed index
/// usages constrained to the target's version, which needs the target's
/// manifest, whereas a view keeps the path.
///
/// A typed usage carries `publisher` and `name` exactly as the manifest
/// spells them, without normalization; the identity check when the usage
/// resolves normalizes both sides.
pub fn usage(usage: &Usage) -> InterchangeProjectUsageRaw {
    match usage {
        Usage::Index {
            publisher,
            name,
            version_constraint,
        } => InterchangeProjectUsageRaw::Index {
            publisher: publisher.as_str().to_owned(),
            name: name.as_str().to_owned(),
            version_constraint: version_constraint.to_string(),
        },
        // The URL as-is; the kind takes no version constraint.
        Usage::RemoteKpar {
            url,
            publisher: _,
            name: _,
        } => InterchangeProjectUsageRaw::Resource {
            resource: url.as_str().to_owned(),
            version_constraint: None,
        },
        Usage::SourcePath {
            publisher,
            name,
            dir,
        } => InterchangeProjectUsageRaw::Directory {
            dir: dir.as_str().to_owned(),
            publisher: publisher.as_str().to_owned(),
            name: name.as_str().to_owned(),
        },
        Usage::KparPath {
            publisher,
            name,
            kpar_path,
        } => InterchangeProjectUsageRaw::KparPath {
            kpar_path: kpar_path.as_str().to_owned(),
            publisher: publisher.as_str().to_owned(),
            name: name.as_str().to_owned(),
        },
    }
}

/// The project as a `.project.json`.
///
/// The remaining `[project]` keys map onto their counterparts: `name`,
/// `version`, `description`, `license` as-is, `topics` into `topic`,
/// `maintainers` into `maintainer`, `urls.website` into `website`.
/// `urls.repository` and `urls.documentation` have no counterpart in
/// `.project.json` and are dropped. `sources` is not written either: the
/// paths it lists are recoverable from the `.meta.json` maps.
///
/// `publisher` is always written, which is the second place the view and
/// the archive part company: the archive omits it for a project carrying
/// `stdlib`, so that a locally built standard-library KPAR matches the
/// published one. A view that omitted it would be circular: the standard
/// library's own projects use each other by `dir`, and a `dir` target must
/// declare the publisher its usage names.
pub fn info(project: &ResolvedProject) -> InterchangeProjectInfoRaw {
    InterchangeProjectInfoRaw {
        name: project.name.as_str().to_owned(),
        publisher: Some(project.publisher.as_str().to_owned()),
        description: project.description.clone(),
        version: project.version.to_string(),
        license: project.license.as_ref().map(ToString::to_string),
        maintainer: project
            .maintainers
            .iter()
            .map(ToString::to_string)
            .collect(),
        website: project
            .urls
            .website
            .as_ref()
            .map(|website| website.as_str().to_owned()),
        topic: project.topics.clone(),
        // TODO: `located.base` is discarded. For a member that inherits
        // a *group* `dir` usage the base is the workspace root, not the
        // member, and a caller that reads its input usages from here
        // would join the path onto the wrong directory.
        usage: project
            .usages
            .iter()
            .map(|located| usage(&located.usage))
            .collect(),
    }
}

/// The project as a `.meta.json`, given the instant to record and the two
/// maps [`source_maps`] computes.
///
/// `created` is passed in as an instant.
///
/// `includesDerived` and `includesImplied` are omitted: Sysand does not
/// know either.
pub fn meta(
    project: &ResolvedProject,
    created: &jiff::Timestamp,
    index: SymbolIndex,
    checksum: ChecksumMap,
) -> InterchangeProjectMetadataRaw {
    // A project with a `root` writes the Syside shape -- `root` and the flat `exports`
    // array, no `index`. The array is the map's keys, sorted.
    let (index, root, exports) = match &project.root {
        Some(root) => {
            let mut exports: Vec<String> = index.into_keys().collect();
            exports.sort();
            (None, Some(root.as_str().to_owned()), Some(exports))
        }
        None => (Some(index), None, None),
    };
    InterchangeProjectMetadataRaw {
        index,
        root,
        exports,
        created: format_created(created),
        // Always written: a resolved project declares one of the four.
        metamodel: Some(project.metamodel.to_string()),
        includes_derived: None,
        includes_implied: None,
        checksum: Some(checksum),
    }
}

/// The two `.meta.json` maps that have no counterpart in `model.toml`,
/// both computed from the file contents and keyed by the `sources`
/// entries:
///
/// - `checksum`, an entry per source file, keyed by its path, holding the
///   file's SHA256 hash;
/// - `index`, the top level symbols extracted from each source file, each
///   mapped to that file's path; the language is taken from the file
///   extension (KerML or SysML), which `sources` guarantees to be one of
///   the two.
///
/// `read` hands over one source file's contents; nothing here opens a
/// file.
///
/// A symbol declared in two sources maps to the later one; `index` has
/// no room for both.
pub fn source_maps<E>(
    sources: &Sources,
    mut read: impl FnMut(&Utf8UnixPath) -> Result<String, E>,
) -> Result<(SymbolIndex, ChecksumMap), ViewError<E>> {
    let mut index = SymbolIndex::new();
    let mut checksum = ChecksumMap::new();

    for source in sources.iter() {
        let contents = read(source).map_err(ViewError::Read)?;
        checksum.insert(
            source.as_str().to_owned(),
            InterchangeProjectChecksumRaw {
                value: sha256_lowercase_hex(&contents),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            },
        );
        for symbol in top_level_symbols(&source, &contents, None)? {
            index.insert(symbol, source.as_str().to_owned());
        }
    }

    Ok((index, checksum))
}

/// The whole view: [`info`] and [`meta`] over one run of [`source_maps`].
pub fn view<E>(
    project: &ResolvedProject,
    created: &jiff::Timestamp,
    read: impl FnMut(&Utf8UnixPath) -> Result<String, E>,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), ViewError<E>> {
    let (index, checksum) = source_maps(&project.sources, read)?;
    Ok((info(project), meta(project, created, index, checksum)))
}

/// Read a `model.toml` into the manifest model.
///
/// `file` names the document in messages; nothing here opens one. The
/// project is resolved *standalone*: a caller that knows the project is
/// a workspace member has to resolve it itself, against the table it
/// belongs to.
///
/// Warnings are logged.
pub fn resolved_of_text(
    file: &str,
    text: &str,
) -> Result<ResolvedProject, ProjectDeserializationError> {
    resolved_of_text_in(file, text, None)
}

/// [`resolved_of_text`], resolved against the workspace the project is a
/// member of.
///
/// A member's manifest may carry inheritance markers, and `None` refuses
/// every one of them: "`version` inherits from the default group, but
/// the project is not a member of any workspace". A caller that knows
/// which workspace this is passes its table.
pub fn resolved_of_text_in(
    file: &str,
    text: &str,
    table: Option<&WorkspaceTable>,
) -> Result<ResolvedProject, ProjectDeserializationError> {
    let manifest: ProjectManifest = validate_manifest(file, ModelToml::parse(file, text))?
        .into_project(file)
        .map_err(|problem| {
            ProjectDeserializationError::invalid(file, Report::from_iter([problem]))
        })?;
    validate_manifest(file, manifest.resolve(table))
}

/// The KPAR view of a `model.toml`, from its text.
///
/// `read` hands over one source file's contents; nothing here opens
/// anything.
pub fn of_text<E>(
    file: &str,
    text: &str,
    read: impl FnMut(&Utf8UnixPath) -> Result<String, E>,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), ViewError<E>> {
    of_text_in(file, text, None, read)
}

/// [`of_text`], resolved against the workspace the project is a member
/// of. See [`resolved_of_text_in`].
pub fn of_text_in<E>(
    file: &str,
    text: &str,
    table: Option<&WorkspaceTable>,
    read: impl FnMut(&Utf8UnixPath) -> Result<String, E>,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), ViewError<E>> {
    let project = resolved_of_text_in(file, text, table)?;
    view(&project, &VIEW_CREATED, read)
}

/// [`of_text`] for a document named `model.toml`.
pub fn of_model_toml<E>(
    text: &str,
    read: impl FnMut(&Utf8UnixPath) -> Result<String, E>,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), ViewError<E>> {
    of_model_toml_in(text, None, read)
}

/// [`of_model_toml`], resolved against the workspace the project is a
/// member of. See [`resolved_of_text_in`].
pub fn of_model_toml_in<E>(
    text: &str,
    table: Option<&WorkspaceTable>,
    read: impl FnMut(&Utf8UnixPath) -> Result<String, E>,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), ViewError<E>> {
    of_text_in(MODEL_TOML_FILE, text, table, read)
}

#[cfg(test)]
#[path = "./kpar_view_tests.rs"]
mod tests;
