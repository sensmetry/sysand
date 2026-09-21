// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The Python view of project info, where it differs from core's.
//!
//! Core has no index usage yet: an index dependency is the resource usage of
//! `pkg:sysand/<publisher>/<name>`. Python shows it as an index usage, so
//! that the shape of a usage says how the project is found. Everything else
//! passes through unchanged. This view is temporary, until core gains an
//! index usage of its own.

use pyo3::prelude::*;
use sysand_core::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    purl::{normalize_field, parse_sysand_purl},
};

/// `InterchangeProjectUsage` in `_model.py`. Variants are tried in order, so
/// `Index`, the only one without a location key, comes last.
#[derive(FromPyObject, IntoPyObject, Debug, Clone, PartialEq, Eq)]
pub(crate) enum PyUsage {
    #[pyo3(from_item_all)]
    Resource {
        resource: String,
        version_constraint: Option<String>,
    },
    #[pyo3(from_item_all)]
    Directory {
        dir: String,
        publisher: String,
        name: String,
    },
    #[pyo3(from_item_all)]
    KparPath {
        kpar_path: String,
        publisher: String,
        name: String,
    },
    // Note that `publisher` and `name` may be normalized. This is a temporary
    // measure to avoid significant complexity and will not be allowed in this usage
    // type once it's introduced in core
    #[pyo3(from_item_all)]
    Index {
        publisher: String,
        name: String,
        // This is `Option` only because currently this is a wrapper of `Resource`,
        // which also has this as an `Option`. Will be required in core once supported.
        version_constraint: Option<String>,
    },
}

impl From<InterchangeProjectUsageRaw> for PyUsage {
    fn from(usage: InterchangeProjectUsageRaw) -> Self {
        match usage {
            InterchangeProjectUsageRaw::Resource {
                resource,
                version_constraint,
            } => match parse_sysand_purl(&resource) {
                Ok(Some((publisher, name))) => Self::Index {
                    publisher: publisher.to_owned(),
                    name: name.to_owned(),
                    version_constraint,
                },
                // Not a `pkg:sysand` PURL, or a malformed one: the latter
                // names no project an index could resolve.
                Ok(None) | Err(_) => Self::Resource {
                    resource,
                    version_constraint,
                },
            },
            InterchangeProjectUsageRaw::Directory {
                dir,
                publisher,
                name,
            } => Self::Directory {
                dir,
                publisher,
                name,
            },
            InterchangeProjectUsageRaw::KparPath {
                kpar_path,
                publisher,
                name,
            } => Self::KparPath {
                kpar_path,
                publisher,
                name,
            },
        }
    }
}

impl From<PyUsage> for InterchangeProjectUsageRaw {
    fn from(usage: PyUsage) -> Self {
        match usage {
            PyUsage::Resource {
                resource,
                version_constraint,
            } => Self::Resource {
                resource,
                version_constraint,
            },
            PyUsage::Directory {
                dir,
                publisher,
                name,
            } => Self::Directory {
                dir,
                publisher,
                name,
            },
            PyUsage::KparPath {
                kpar_path,
                publisher,
                name,
            } => Self::KparPath {
                kpar_path,
                publisher,
                name,
            },
            // Validating the usage later rejects a publisher or name that
            // normalizes to no valid segment.
            PyUsage::Index {
                publisher,
                name,
                version_constraint,
            } => Self::Resource {
                resource: index_purl(&publisher, &name),
                version_constraint,
            },
        }
    }
}

/// The `pkg:sysand` PURL that stores the index usage of `publisher` and
/// `name`, normalized the way a directory usage's identifier is. Neither is
/// validated: a value that normalizes to no valid segment gives a PURL that
/// validation rejects.
pub(crate) fn index_purl(publisher: &str, name: &str) -> String {
    format!(
        "pkg:sysand/{}/{}",
        normalize_field(publisher),
        normalize_field(name)
    )
}

/// `InterchangeProjectInfo` in `_model.py`: core's info, with `usage` in the
/// Python view.
#[derive(FromPyObject, IntoPyObject, Debug, Clone, PartialEq, Eq)]
#[pyo3(from_item_all)]
pub(crate) struct PyInfo {
    name: String,
    publisher: Option<String>,
    description: Option<String>,
    version: String,
    license: Option<String>,
    maintainer: Vec<String>,
    website: Option<String>,
    topic: Vec<String>,
    usage: Vec<PyUsage>,
}

impl From<InterchangeProjectInfoRaw> for PyInfo {
    fn from(info: InterchangeProjectInfoRaw) -> Self {
        let InterchangeProjectInfoRaw {
            name,
            publisher,
            description,
            version,
            license,
            maintainer,
            website,
            topic,
            usage,
        } = info;
        Self {
            name,
            publisher,
            description,
            version,
            license,
            maintainer,
            website,
            topic,
            usage: usage.into_iter().map(PyUsage::from).collect(),
        }
    }
}

impl From<PyInfo> for InterchangeProjectInfoRaw {
    fn from(info: PyInfo) -> Self {
        let PyInfo {
            name,
            publisher,
            description,
            version,
            license,
            maintainer,
            website,
            topic,
            usage,
        } = info;
        Self {
            name,
            publisher,
            description,
            version,
            license,
            maintainer,
            website,
            topic,
            usage: usage
                .into_iter()
                .map(InterchangeProjectUsageRaw::from)
                .collect(),
        }
    }
}
