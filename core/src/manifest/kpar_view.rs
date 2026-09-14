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
//! function of one manifest, which is what a read-only consumer of a
//! project needs, and it is free of Cargo features so that the bindings
//! without a filesystem can produce it too.

use indexmap::IndexMap;
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::manifest::common::Sources;
use crate::manifest::project::ResolvedProject;
use crate::manifest::usage::Usage;
use crate::model::{
    InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
    InterchangeProjectUsageRaw, KerMlChecksumAlg, format_created,
};
use crate::symbols::{SymbolsError, top_level_symbols};
use crate::utils::sha256_lowercase_hex;

/// `index`, as a `.meta.json` carries it: every top level symbol mapped
/// onto the source file that declares it.
pub type SymbolIndex = IndexMap<String, String>;

/// `checksum`, as a `.meta.json` carries it: one entry per source file.
pub type ChecksumMap = IndexMap<String, InterchangeProjectChecksumRaw>;

/// Why a view could not be built.
#[derive(Debug, Error)]
pub enum ViewError<E> {
    /// The caller could not hand over a source file's contents. Passed
    /// through rather than wrapped: the caller's error already names the
    /// file it failed on, and a second sentence would print the path
    /// twice.
    #[error(transparent)]
    Read(E),
    #[error(transparent)]
    Symbols(#[from] SymbolsError),
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
/// match the values found at the target". A PURL is built from the
/// normalized pair instead, and cannot fail: `Publisher` and `Name`
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
        Usage::RemoteKpar { url, .. } => InterchangeProjectUsageRaw::Resource {
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
        metamodel: project.metamodel.clone(),
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

#[cfg(test)]
#[path = "./kpar_view_tests.rs"]
mod tests;
