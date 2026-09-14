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

use thiserror::Error;

use crate::manifest::usage::Usage;
use crate::model::InterchangeProjectUsageRaw;
use crate::symbols::SymbolsError;

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

#[cfg(test)]
#[path = "./kpar_view_tests.rs"]
mod tests;
