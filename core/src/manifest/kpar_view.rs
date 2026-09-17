// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The KPAR view of a `model.toml` project: the `.project.json` and
//! `.meta.json` pair a reader is handed when it asks such a project for
//! its manifest.
//!
//! model-toml.md "TOML to KPAR" specifies the pair a *build* writes into
//! an archive, and the view follows it wherever it can. Three rules are
//! the archive's alone, and each is marked "the archive" below:
//!
//! 1. `publisher` is omitted from a `stdlib` project's `.project.json`;
//! 2. `dir` and `kpar-path` usages become PURLs constrained to
//!    `^<target version>`;
//! 3. a usage whose target carries `stdlib` becomes an OMG IRI.
//!
//! All three need something this module does not have -- the project's
//! targets -- so all three belong to the build. What is left is a
//! function of one manifest, which is what every read-only consumer of a
//! project needs, and it is free of Cargo features so that the bindings
//! without a filesystem can produce it too.

use fluent_uri::Iri;
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
/// A build records the instant it ran, so that two builds of identical
/// sources are not byte-identical. A *view* must not: the project
/// checksum covers the whole `.meta.json`, and that checksum is written
/// into the lockfile and recomputed and compared by `sysand sync`. A
/// view is a function of the manifest and the source files; the clock is
/// not one of its inputs.
pub const VIEW_CREATED: jiff::Timestamp = jiff::Timestamp::UNIX_EPOCH;

/// Why a view could not be built.
#[derive(Debug, Error)]
pub enum ViewError<E> {
    /// The `model.toml` is not one, or does not define a project, or
    /// does not validate.
    #[error(transparent)]
    Manifest(#[from] ProjectDeserializationError),
    /// The caller could not hand over a source file's contents. Passed
    /// through rather than wrapped: the caller's error already names the
    /// file it failed on, and a second sentence would print the path
    /// twice.
    #[error(transparent)]
    Read(E),
    #[error(transparent)]
    Symbols(#[from] SymbolsError),
}

/// Why a json usage has no `model.toml` form.
///
/// Three different refusals, because a caller has three different
/// things to tell the user. A single "cannot be expressed" would be
/// useless: one is fixed by typing a constraint, one by editing the
/// target's `.project.json`, and one cannot be fixed at all in v1.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NoManifestForm {
    /// model-toml.md "Usages": an index usage "requires a `version`
    /// constraint", and a `pkg:sysand` PURL carries none.
    ///
    /// The specification resolves this at migration time by asking the
    /// index for the version and pinning `^<version>`. Inside `sysand
    /// add` that would be a network call in the middle of a manifest
    /// edit, so the constraint is asked for instead.
    #[error(
        "a `model.toml` index usage needs a version constraint;\n\
        {SP:>8} try `sysand add {publisher}/{name} <constraint>`",
        SP = ' '
    )]
    NoConstraint { publisher: String, name: String },
    /// An untyped usage that is not a `pkg:sysand` PURL. model-toml.md:
    /// identity in a manifest "is always publisher+name", and there is
    /// nothing here to take them from.
    #[error(
        "`{0}` cannot be written into a `model.toml`:\n\
        {SP:>8} a manifest usage is named by its publisher and name, which this IRI does not carry",
        SP = ' '
    )]
    UntypedIri(String),
    /// A `kpar-url` usage takes no version constraint, and a
    /// standard-library usage is written as one.
    #[error(
        "`{0}` cannot be written into a `model.toml` with a version constraint:\n\
        {SP:>8} it is a standard-library IRI, and a `kpar-url` usage takes no version constraint",
        SP = ' '
    )]
    StdlibConstraint(String),
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
/// The reverse of [`usage`], and deliberately not its inverse: the
/// forward map writes an index usage as the *normalized* PURL, so a
/// publisher written `ACME` comes back `acme`; and it turns a `kpar-url`
/// usage into a bare IRI, dropping the publisher and name a manifest
/// needs. Nothing can therefore produce a `kpar-url` entry from json, which
/// is a gap in v1 rather than in this function.
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
            version_constraint,
        } => {
            // model-toml.md "Standard library": "A json project's
            // `resource` usage of such an IRI is the same usage under
            // the same spelling" -- the publisher-less `kpar-url` form.
            if let Some(stem) = crate::model::omg_stdlib_stem(resource) {
                if version_constraint.is_some() {
                    return Err(NoManifestForm::StdlibConstraint(resource.clone()));
                }
                let name = IndexName::parse(stem).map_err(|e| NoManifestForm::Identifier {
                    file: file.to_owned(),
                    problem: format!("`{}`: {e}", e.value),
                })?;
                let url = Iri::parse(resource.to_owned()).map_err(|(e, _)| {
                    NoManifestForm::Identifier {
                        file: file.to_owned(),
                        problem: format!("`{resource}`: {e}"),
                    }
                })?;
                return Ok(Usage::RemoteKpar {
                    publisher: None,
                    name,
                    url,
                });
            }
            let Ok(Some((publisher, name))) = crate::purl::parse_sysand_purl(resource) else {
                return Err(NoManifestForm::UntypedIri(resource.clone()));
            };
            let Some(constraint) = version_constraint else {
                return Err(NoManifestForm::NoConstraint {
                    publisher: publisher.to_owned(),
                    name: name.to_owned(),
                });
            };
            let (publisher, name) = identity(publisher, name)?;
            let version_constraint =
                semver::VersionReq::parse(constraint).map_err(|e| NoManifestForm::Identifier {
                    file: file.to_owned(),
                    problem: format!("version constraint `{constraint}`: {e}"),
                })?;
            Ok(Usage::Index {
                publisher,
                name,
                version_constraint,
            })
        }
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
/// model-toml.md "TOML to KPAR": an index usage becomes a PURL with the
/// constraint as declared, and a remote-kpar usage its URL as-is. The
/// two local kinds become *typed* usages here, which is where the view
/// and the archive part company: the archive rewrites them into PURLs
/// constrained to the target's version at build time, which needs the
/// target's manifest, and the resolver needs the path in any case (a
/// `model.toml` `dir` usage always resolves to an editable project).
///
/// A typed usage "carries `publisher` and `name` exactly as the manifest
/// spells them, without normalization, because Sysand requires them to
/// match the values found at the target". That is about what is
/// *written*; the identity check when the usage resolves normalizes
/// both sides, as model-toml.md "Normalization" requires. A PURL is built from the
/// normalized pair instead, and cannot fail: `IndexPublisher` and `IndexName`
/// accept only values that are valid PURL segments once normalized (see
/// `purl::is_valid_unnormalized_publisher`), so the identity a usage
/// holds is always encodable. It is spelled by the same function the
/// lockfile uses, so the two cannot drift apart.
pub fn usage(usage: &Usage) -> InterchangeProjectUsageRaw {
    match usage {
        Usage::Index {
            publisher,
            name,
            version_constraint,
        } => InterchangeProjectUsageRaw::Resource {
            resource: crate::manifest::common::normalized_identity(publisher, name)
                .lock_identifier(),
            version_constraint: Some(version_constraint.to_string()),
        },
        // "remote-kpar: use the URL as-is"; the kind takes no version
        // constraint.
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
/// model-toml.md "TOML to KPAR": "the remaining `[project]` keys map onto
/// their counterparts: `name`, `version`, `description`, `license` as-is,
/// `topics` into `topic`, `maintainers` into `maintainer`, `urls.website`
/// into `website`. `urls.repository` and `urls.documentation` have no
/// counterpart in `.project.json` and are dropped". `sources` is not
/// written either: "the paths it lists are recoverable from these two
/// maps" of the `.meta.json`.
///
/// `publisher` is always written, which is the second place the view and
/// the archive part company: the archive omits it for a project carrying
/// `stdlib`, so that a locally built standard-library KPAR matches the
/// published one. A view that omitted it would be worse than wrong, it
/// would be circular -- the standard library's own projects use each
/// other by `dir`, and a `dir` target must declare the publisher its
/// usage names, so the standard library could not build itself. Lock
/// entries would lose it too.
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
/// `created` is an instant rather than a string so that it is written the
/// one way `InterchangeProjectMetadataRaw` is documented to be written,
/// and a parameter rather than the current time so that what a view says
/// can be asserted.
///
/// model-toml.md "TOML to KPAR": `includesDerived` and `includesImplied`
/// "are optional and omitted" -- Sysand does not know either.
pub fn meta(
    project: &ResolvedProject,
    created: &jiff::Timestamp,
    index: SymbolIndex,
    checksum: ChecksumMap,
) -> InterchangeProjectMetadataRaw {
    InterchangeProjectMetadataRaw {
        index,
        created: format_created(created),
        // Always written: a resolved project declares one of the
        // four (model-toml.md "Metamodel").
        metamodel: Some(project.metamodel.to_string()),
        includes_derived: None,
        includes_implied: None,
        checksum: Some(checksum),
    }
}

/// The two `.meta.json` maps model-toml.md says have "no counterpart in
/// `model.toml`, both computed at build time from the file contents and
/// keyed by the normalized `sources` entries":
///
/// - `checksum`, "an entry per source file, keyed by its path, holding
///   the file's hash (SHA256, as `sysand include` already produces)";
/// - `index`, "the top level symbols extracted from each source file,
///   each mapped to that file's path; the language is taken from the file
///   extension (KerML or SysML), which `sources` guarantees to be one of
///   the two".
///
/// `read` hands over one source file's contents. Nothing here opens a
/// file, which is what keeps this module free of Cargo features and lets
/// a binding with no filesystem produce the same view.
///
/// Both maps are built in `sources` order, so that a `.meta.json` is a
/// deterministic function of the manifest and the file contents.
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
/// Warnings are logged rather than returned, which is what the json
/// readers do with theirs, so that a manifest warns the same way
/// whichever format it is written in.
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
/// This is the whole of what a toml-backed project answers with, and it
/// is free of Cargo features so that a store without a filesystem
/// produces the same answer: `read` hands over one source file's
/// contents and nothing here opens anything.
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

/// [`of_text`] for a document named `model.toml`, which every source
/// project's is.
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
