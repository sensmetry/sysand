// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

//! Core types for interchange projects. Originally matched KerML 1.0 spec,
//! but now includes various changes.
//!
//! IMPORTANT: when updating any of these types, update the corresponding
//! bindings' types (in `_model.py` for Python and various Java classes)

use std::{clone::Clone, collections::HashSet, fmt::Display, hash::Hash};

use digest::array::{Array, typenum};
use fluent_uri::Iri;
use icu_properties::props::GeneralCategoryGroup;
use indexmap::IndexMap;
#[cfg(feature = "python")]
use pyo3::{FromPyObject, IntoPyObject, pyclass};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::purl::parse_sysand_purl;
use crate::utils::{
    CASE_MAPPER, GENERAL_CATEGORY, IGNORABLE, NFKC_NORMALIZER, NFKD_NORMALIZER,
    PROJECT_FIELD_ASCII_PUNCTUATION, PURL_NAME_SEPARATOR, PURL_SEPARATOR, RelativePathKind,
    RelativeUnixPathError, UNNORMALIZED_PURL_SEPARATOR, XID_CONTINUE, lowercase_hex,
    parse_relative_unix_path,
};

// pub struct RawIri(String);
// pub struct ParsedIri(fluent_uri::Iri<String>);
// pub struct NormalisedIri(fluent_uri::Iri<String>);

pub const KNOWN_METAMODELS: [&str; 2] = [
    "https://www.omg.org/spec/SysML/20250201",
    "https://www.omg.org/spec/KerML/20250201",
];

/// Prefix shared between SysML v2 metamodel and standard libs
pub const SYSML_SPEC_PREFIX: &str = "https://www.omg.org/spec/SysML/";
/// Prefix shared between KerML metamodel and standard libs
pub const KERML_SPEC_PREFIX: &str = "https://www.omg.org/spec/KerML/";

pub const LICENSE_EXPRESSION_HELP: &str = "\
    see https://spdx.github.io/spdx-spec/v3.0.1/annexes/spdx-license-expressions/\n\
    for the syntax and https://spdx.org/licenses/ for the list of license identifiers;\n\
    for custom licenses, use `LicenseRef-My-custom-license`";

/// A dependency on another interchange project. In the dependency solver,
/// usages are treated as referring to the same project iff they derive the
/// same `Identifier`. A solution contains one instance per identifier, which
/// all usages of that identifier accept (in particular, it satisfies
/// all version constraints)
///
/// A typed usage (any kind but [`Self::Resource`]) names its project by
/// `publisher` and `name`, which must match the project's own exactly,
/// without any normalization; locking checks this.
///
/// `.project.json` stores every kind as a bare object with no kind key, so
/// the kind is decided by which keys are present (see [`Usage`] for how).
#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Hash, Debug)]
#[cfg_attr(
    feature = "python",
    derive(FromPyObject, IntoPyObject),
    pyo3(from_item_all)
)]
#[serde(
    untagged,
    from = "Usage<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>"
)]
pub enum InterchangeProjectUsageG<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName> {
    /// Untyped usage, the only shape KerML 1.0 specifies. Kept for
    /// compatibility with the spec. `resource` serves two roles at once: it is
    /// the project's identity (i.e. directly used as its `Identifier`), and,
    /// depending on its scheme, possibly also a location to fetch from (e.g.
    /// `pkg:sysand` vs `https`).
    /// Resolution is not defined, so it's implementation-specific.
    /// We generally treat the same project referenced via different IRIs as
    /// different projects. Typed usages (all other shapes) separate the two roles:
    /// identity is always publisher+name, and the source is explicit.
    // `rename_all` does not apply to enum variant fields if applied on
    // the whole enum
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "python", pyo3(from_item_all))]
    Resource {
        resource: Iri, // TODO: We should have a fallback for invalid IRIs
        #[serde(skip_serializing_if = "Option::is_none")]
        version_constraint: Option<VersionReq>,
    },
    /// The project in the directory `dir`, relative to the root
    /// of the project declaring the usage.
    /// No version constraint, as the directory contains a single version
    // TODO: should absolute paths also be supported (like Cargo)?
    #[cfg_attr(feature = "python", pyo3(from_item_all))]
    Directory {
        dir: Path,
        publisher: Publisher,
        name: Name,
    },
    /// The project KPAR at `kpar_path`, relative to the root of
    /// the project declaring the usage.
    /// Project must be at the archive root
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "python", pyo3(from_item_all))]
    KparPath {
        kpar_path: Path,
        publisher: Publisher,
        name: Name,
    },
    /// The project `publisher`/`name` from the configured indexes, or from
    /// any other source that resolves by identity (the local environment,
    /// workspace members).
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "python", pyo3(from_item_all))]
    Index {
        publisher: IdxPublisher,
        name: IdxName,
        version_constraint: VersionReq,
    },
}

/// Parse how a directory or KPAR usage spells its project, as
/// `publisher`/`name`
fn parse_project_usage_spelling(
    publisher: &str,
    name: &str,
) -> Result<(ProjectPublisher, ProjectName), InterchangeProjectValidationError> {
    let project_publisher =
        ProjectPublisher::parse(publisher.to_owned()).map_err(|(publisher, source)| {
            InterchangeProjectValidationError::InvalidUsagePublisher {
                publisher,
                name: name.to_owned(),
                source,
            }
        })?;
    let project_name = ProjectName::parse(name.to_owned()).map_err(|(name, source)| {
        InterchangeProjectValidationError::InvalidUsageName {
            publisher: publisher.to_owned(),
            name,
            source,
        }
    })?;
    Ok((project_publisher, project_name))
}

/// Parse how an index usage spells its project, as `publisher`/`name`.
/// Any other spelling would make the usage's identifier fall back to a
/// percent-encoded IRI that no index routes
pub fn parse_index_usage_spelling(
    publisher: &str,
    name: &str,
) -> Result<(IndexPublisher, IndexName), InterchangeProjectValidationError> {
    let index_publisher =
        IndexPublisher::parse(publisher.to_owned()).map_err(|(publisher, _)| {
            InterchangeProjectValidationError::InvalidIndexUsagePublisher {
                publisher,
                name: name.to_owned(),
            }
        })?;
    let index_name = IndexName::parse(name.to_owned()).map_err(|(name, _)| {
        InterchangeProjectValidationError::InvalidIndexUsageName {
            publisher: publisher.to_owned(),
            name,
        }
    })?;
    Ok((index_publisher, index_name))
}

/// How [`InterchangeProjectUsageG`] is read. The variants are tried in
/// declaration order, and the first one that accepts the keys present wins.
///
/// Every typed kind rejects a key it does not know (`deny_unknown_fields`,
/// which serde only offers per struct, hence the structs here). Without a
/// kind key, an unknown key could otherwise change what an entry means
/// unnoticed: a future kind that adds a source key (say, `git`) to an index
/// usage's keys would be read as an index usage, and a future
/// `versionConstraint` on a directory usage would be dropped. Such an entry
/// matches no kind and fails to parse instead.
///
/// A resource usage, the shape KerML specifies, keeps ignoring unknown keys,
/// as manifests written for sysand before typed usages rely on it. Its
/// `resource` key names its source, so a key it ignores cannot turn it into
/// another kind.
#[derive(Deserialize)]
#[serde(untagged)]
enum Usage<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName> {
    Resource(ResourceUsage<Iri, VersionReq>),
    Directory(DirectoryUsage<Path, Publisher, Name>),
    KparPath(KparPathUsage<Path, Publisher, Name>),
    Index(IndexUsage<VersionReq, IdxPublisher, IdxName>),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceUsage<Iri, VersionReq> {
    resource: Iri,
    version_constraint: Option<VersionReq>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryUsage<Path, Publisher, Name> {
    dir: Path,
    publisher: Publisher,
    name: Name,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KparPathUsage<Path, Publisher, Name> {
    kpar_path: Path,
    publisher: Publisher,
    name: Name,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IndexUsage<VersionReq, IdxPublisher, IdxName> {
    publisher: IdxPublisher,
    name: IdxName,
    version_constraint: VersionReq,
}

impl<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>
    From<Usage<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>>
    for InterchangeProjectUsageG<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>
{
    fn from(usage: Usage<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>) -> Self {
        match usage {
            Usage::Resource(ResourceUsage {
                resource,
                version_constraint,
            }) => Self::Resource {
                resource,
                version_constraint,
            },
            Usage::Directory(DirectoryUsage {
                dir,
                publisher,
                name,
            }) => Self::Directory {
                dir,
                publisher,
                name,
            },
            Usage::KparPath(KparPathUsage {
                kpar_path,
                publisher,
                name,
            }) => Self::KparPath {
                kpar_path,
                publisher,
                name,
            },
            Usage::Index(IndexUsage {
                publisher,
                name,
                version_constraint,
            }) => Self::Index {
                publisher,
                name,
                version_constraint,
            },
        }
    }
}

pub type InterchangeProjectUsageRaw =
    InterchangeProjectUsageG<String, String, String, String, String, String, String>;
pub type InterchangeProjectUsage = InterchangeProjectUsageG<
    fluent_uri::Iri<String>,
    semver::VersionReq,
    Utf8UnixPathBuf,
    ProjectPublisher,
    ProjectName,
    IndexPublisher,
    IndexName,
>;

impl InterchangeProjectUsageRaw {
    /// Caller is responsible for identifying the project when reporting the error
    pub fn validate(&self) -> Result<InterchangeProjectUsage, InterchangeProjectValidationError> {
        match self {
            Self::Resource {
                resource,
                version_constraint,
            } => {
                // `pkg:sysand/<publisher>/<name>` is the canonical sysand project
                // identifier; the index protocol routes it directly under
                // `<publisher>/<name>/`. Reject malformed or non-normalized
                // `pkg:sysand` IRIs at validation time so users get an actionable
                // error (with a suggested normalized form) instead of a downstream
                // "not found" — see `crate::purl::parse_sysand_purl`.
                crate::purl::parse_sysand_purl(resource).map_err(|e| {
                    InterchangeProjectValidationError::MalformedUsageSysandPurl {
                        iri: resource.clone(),
                        source: e,
                    }
                })?;
                // `urn:sysand` identifiers are internal to Sysand; a project
                // is used by its publisher and name instead
                if crate::project::utils::is_urn_sysand(resource) {
                    return Err(InterchangeProjectValidationError::UrnSysandUsage(
                        resource.clone(),
                    ));
                }

                Ok(InterchangeProjectUsage::Resource {
                    resource: fluent_uri::Iri::parse(resource.clone()).map_err(|(e, val)| {
                        InterchangeProjectValidationError::InvalidUsageResource(val, e)
                    })?,

                    version_constraint: version_constraint
                        .as_ref()
                        .map(|c| {
                            semver::VersionReq::parse(c).map_err(|e| {
                                InterchangeProjectValidationError::InvalidUsageVersionConstraint {
                                    resource: resource.to_owned(),
                                    constraint: c.to_owned(),
                                    source: e,
                                }
                            })
                        })
                        .transpose()?,
                })
            }
            Self::Directory {
                dir: path,
                publisher,
                name,
            } => match parse_relative_unix_path(path, RelativePathKind::Directory) {
                Ok(p) => {
                    let (publisher, name) = parse_project_usage_spelling(publisher, name)?;
                    Ok(InterchangeProjectUsage::Directory {
                        dir: p.to_owned(),
                        publisher,
                        name,
                    })
                }
                Err(e) => Err(InterchangeProjectValidationError::InvalidUsagePath {
                    publisher: publisher.clone(),
                    name: name.clone(),
                    source: e,
                }),
            },
            Self::KparPath {
                kpar_path,
                publisher,
                name,
            } => match parse_relative_unix_path(kpar_path, RelativePathKind::File) {
                Ok(p) => {
                    let (publisher, name) = parse_project_usage_spelling(publisher, name)?;
                    Ok(InterchangeProjectUsage::KparPath {
                        kpar_path: p.to_owned(),
                        publisher,
                        name,
                    })
                }
                Err(e) => Err(InterchangeProjectValidationError::InvalidUsagePath {
                    publisher: publisher.clone(),
                    name: name.clone(),
                    source: e,
                }),
            },
            Self::Index {
                publisher,
                name,
                version_constraint,
            } => {
                let (index_publisher, index_name) = parse_index_usage_spelling(publisher, name)?;
                let version_constraint =
                    semver::VersionReq::parse(version_constraint).map_err(|e| {
                        InterchangeProjectValidationError::InvalidIndexUsageVersionConstraint {
                            publisher: publisher.clone(),
                            name: name.clone(),
                            constraint: version_constraint.clone(),
                            source: e,
                        }
                    })?;
                Ok(InterchangeProjectUsage::Index {
                    publisher: index_publisher,
                    name: index_name,
                    version_constraint,
                })
            }
        }
    }
}

impl<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>
    InterchangeProjectUsageG<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>
{
    /// Typed usages (all non-`Resource`) are treated specially in some places,
    /// as e.g. if they resolve to invalid projects, it can't be ignored
    pub fn is_typed(&self) -> bool {
        !matches!(self, Self::Resource { .. })
    }

    /// The `publisher` and `name` of a typed usage; `None` for a resource
    /// usage
    pub fn typed_publisher_name(&self) -> Option<(&str, &str)>
    where
        Publisher: AsRef<str>,
        Name: AsRef<str>,
        IdxPublisher: AsRef<str>,
        IdxName: AsRef<str>,
    {
        match self {
            Self::Resource { .. } => None,
            Self::Directory {
                publisher, name, ..
            }
            | Self::KparPath {
                publisher, name, ..
            } => Some((publisher.as_ref(), name.as_ref())),
            Self::Index {
                publisher, name, ..
            } => Some((publisher.as_ref(), name.as_ref())),
        }
    }

    /// A short noun naming this usage's kind, with its indefinite article
    /// (e.g. "an index"), for error messages.
    pub fn kind_with_article(&self) -> &'static str {
        match self {
            Self::Resource { .. } => "a resource",
            Self::Directory { .. } => "a directory",
            Self::KparPath { .. } => "a KPAR path",
            Self::Index { .. } => "an index",
        }
    }
}

impl From<InterchangeProjectUsage> for InterchangeProjectUsageRaw {
    fn from(value: InterchangeProjectUsage) -> Self {
        match value {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint,
            } => Self::Resource {
                resource: resource.into_string(),
                version_constraint: version_constraint.map(|x| x.to_string()),
            },
            InterchangeProjectUsage::Directory {
                dir,
                publisher,
                name,
            } => Self::Directory {
                dir: dir.into_string(),
                publisher: publisher.into_string(),
                name: name.into_string(),
            },
            InterchangeProjectUsage::KparPath {
                kpar_path,
                publisher,
                name,
            } => Self::KparPath {
                kpar_path: kpar_path.into_string(),
                publisher: publisher.into_string(),
                name: name.into_string(),
            },
            InterchangeProjectUsage::Index {
                publisher,
                name,
                version_constraint,
            } => Self::Index {
                publisher: publisher.into_string(),
                name: name.into_string(),
                version_constraint: version_constraint.to_string(),
            },
        }
    }
}

impl From<InterchangeProjectUsage>
    for InterchangeProjectUsageG<
        String,
        semver::VersionReq,
        Utf8UnixPathBuf,
        ProjectPublisher,
        ProjectName,
        IndexPublisher,
        IndexName,
    >
{
    fn from(value: InterchangeProjectUsage) -> Self {
        match value {
            InterchangeProjectUsageG::Resource {
                resource,
                version_constraint,
            } => Self::Resource {
                resource: resource.into_string(),
                version_constraint,
            },
            InterchangeProjectUsage::Directory {
                dir,
                publisher,
                name,
            } => Self::Directory {
                dir,
                publisher,
                name,
            },
            InterchangeProjectUsageG::KparPath {
                kpar_path,
                publisher,
                name,
            } => Self::KparPath {
                kpar_path,
                publisher,
                name,
            },
            InterchangeProjectUsageG::Index {
                publisher,
                name,
                version_constraint,
            } => Self::Index {
                publisher,
                name,
                version_constraint,
            },
        }
    }
}

impl<
    Iri: Display,
    VersionReq: Display,
    Path: Display,
    Publisher: Display,
    Name: Display,
    IdxPublisher: Display,
    IdxName: Display,
> Display
    for InterchangeProjectUsageG<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Resource {
                resource,
                version_constraint,
            } => {
                write!(f, "IRI `{resource}`")?;
                if let Some(vc) = version_constraint {
                    write!(f, " ({vc})")?;
                }
            }
            Self::Directory {
                dir,
                publisher,
                name,
            } => {
                write!(f, "`{publisher}/{name}` from `{dir}`")?;
            }
            Self::KparPath {
                kpar_path,
                publisher,
                name,
            } => {
                write!(f, "`{publisher}/{name}` in `{kpar_path}`")?;
            }
            Self::Index {
                publisher,
                name,
                version_constraint,
            } => {
                write!(f, "`{publisher}/{name}` ({version_constraint})")?;
            }
        }
        Ok(())
    }
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(
    feature = "python",
    derive(FromPyObject, IntoPyObject),
    pyo3(from_item_all)
)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectInfoG<
    Iri,
    Version,
    License,
    VersionReq,
    Path,
    Name,
    Publisher,
    IdxPublisher,
    IdxName,
> {
    pub name: Name,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<Publisher>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    pub version: Version,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub maintainer: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<Iri>, // TODO We should have a fallback for invalid IRIs

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub topic: Vec<String>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub usage: Vec<
        InterchangeProjectUsageG<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>,
    >,
}

pub type InterchangeProjectInfoRaw =
    InterchangeProjectInfoG<String, String, String, String, String, String, String, String, String>;
pub type InterchangeProjectInfo = InterchangeProjectInfoG<
    fluent_uri::Iri<String>,
    semver::Version,
    spdx::Expression,
    semver::VersionReq,
    Utf8UnixPathBuf,
    ProjectName,
    ProjectPublisher,
    IndexPublisher,
    IndexName,
>;

impl From<InterchangeProjectInfo> for InterchangeProjectInfoRaw {
    fn from(value: InterchangeProjectInfo) -> Self {
        InterchangeProjectInfoRaw {
            name: value.name.into_string(),
            publisher: value.publisher.map(ProjectPublisher::into_string),
            description: value.description,
            version: value.version.to_string(),
            license: value.license.map(|l| l.to_string()),
            maintainer: value.maintainer,
            website: value.website.map(|uri| uri.to_string()),
            topic: value.topic,
            usage: value
                .usage
                .iter()
                .map(|u| From::from(u.to_owned()))
                .collect(),
        }
    }
}

/// Reference to a project, by IRI or by publisher and name
#[derive(Debug, Clone, Copy)]
pub enum UsageRef<'a> {
    /// The project identified by this IRI
    Resource(Iri<&'a str>),
    /// The project with this `publisher` and `name`
    Typed(&'a str, &'a str),
}

impl UsageRef<'_> {
    /// Whether `usage` refers to this project. A `Resource` matches a
    /// resource usage of exactly this IRI. `Typed` matches a typed usage
    /// whose publisher and name are each given either exactly as declared,
    /// or normalized (see [`normalize_typed_publisher`] and
    /// [`normalize_typed_name`]); it also matches a `pkg:sysand` resource
    /// usage in any spelling that normalizes to it, as the PURL only holds
    /// the normalized form
    pub fn matches<
        Iri: AsRef<str>,
        VersionReq,
        Path,
        Publisher: AsRef<str>,
        Name: AsRef<str>,
        IdxPublisher: AsRef<str>,
        IdxName: AsRef<str>,
    >(
        &self,
        usage: &InterchangeProjectUsageG<
            Iri,
            VersionReq,
            Path,
            Publisher,
            Name,
            IdxPublisher,
            IdxName,
        >,
    ) -> bool {
        match self {
            Self::Resource(iri) => matches!(
                usage,
                InterchangeProjectUsageG::Resource { resource, .. } if resource.as_ref() == iri.as_str()
            ),
            Self::Typed(publisher, name) => {
                if let InterchangeProjectUsageG::Resource { resource, .. } = usage {
                    // A `pkg:sysand` IRI only holds the normalized form
                    return parse_sysand_purl(resource.as_ref()).is_ok_and(|parsed| {
                        parsed.is_some_and(|(p, n)| {
                            p == normalize_typed_publisher(publisher)
                                && n == normalize_typed_name(name)
                        })
                    });
                }
                let (p, n) = usage
                    .typed_publisher_name()
                    .expect("a non-resource usage is typed");
                (*publisher == p || *publisher == normalize_typed_publisher(p))
                    && (*name == n || *name == normalize_typed_name(n))
            }
        }
    }
}

impl Display for UsageRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Resource(iri) => write!(f, "{iri}"),
            Self::Typed(publisher, name) => write!(f, "{publisher}/{name}"),
        }
    }
}

impl<
    Iri: PartialEq + Clone,
    Version,
    License,
    VersionReq: Clone,
    Path,
    Name,
    Publisher,
    IdxPublisher,
    IdxName,
>
    InterchangeProjectInfoG<
        Iri,
        Version,
        License,
        VersionReq,
        Path,
        Name,
        Publisher,
        IdxPublisher,
        IdxName,
    >
{
    pub fn minimal(name: Name, version: Version) -> Self {
        Self {
            name,
            publisher: None,
            description: None,
            version,
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        }
    }

    /// Remove and return all usages matching `usage`.
    /// Note that sysand will never add multiple usages of the same resource
    /// to the project, but it does tolerate such usages.
    // TODO: the spec does not say anything about this and should be clarified
    pub fn pop_usage(
        &mut self,
        usage: &UsageRef<'_>,
    ) -> Vec<InterchangeProjectUsageG<Iri, VersionReq, Path, Publisher, Name, IdxPublisher, IdxName>>
    where
        Iri: AsRef<str>,
        Publisher: AsRef<str>,
        Name: AsRef<str>,
        IdxPublisher: AsRef<str>,
        IdxName: AsRef<str>,
    {
        self.usage.extract_if(.., |u| usage.matches(u)).collect()
    }
}

impl InterchangeProjectInfoRaw {
    /// Caller is responsible for identifying the project when reporting the error
    pub fn validate(&self) -> Result<InterchangeProjectInfo, InterchangeProjectValidationError> {
        let mut usage = vec![];
        for a_usage in &self.usage {
            usage.push(a_usage.to_owned().validate()?);
        }

        Ok(InterchangeProjectInfo {
            name: ProjectName::parse(self.name.clone()).map_err(|(name, e)| {
                InterchangeProjectValidationError::InvalidProjectName(name.into(), e)
            })?,
            publisher: self
                .publisher
                .clone()
                .map(ProjectPublisher::parse)
                .transpose()
                .map_err(|(publisher, e)| {
                    InterchangeProjectValidationError::InvalidProjectPublisher(publisher.into(), e)
                })?,
            description: self.description.clone(),
            version: semver::Version::parse(&self.version).map_err(|e| {
                InterchangeProjectValidationError::InvalidProjectVersion(
                    self.version.as_str().into(),
                    e,
                )
            })?,
            license: match self.license.as_deref() {
                Some(l) => {
                    let license = spdx::Expression::parse(l)
                        .map_err(InterchangeProjectValidationError::InvalidProjectLicense)?;
                    Some(license)
                }
                None => None,
            },
            maintainer: self.maintainer.clone(),
            website: self
                .website
                .clone()
                .map(fluent_uri::Iri::parse)
                .transpose()
                .map_err(|(e, val)| InterchangeProjectValidationError::InvalidWebsite(val, e))?,

            topic: self.topic.clone(),
            usage,
        })
    }
}

/// KerML 1.0, 10.3 note 6, page 409:
/// Valid values for the checksum algorithm are:
/// - SHA1, SHA224, SHA256, SHA-384, SHA3-256, SHA3-384, SHA3-512
/// - BLAKE2b-256, BLAKE2b-384, BLAKE2b-512, BLAKE3
/// - MD2, MD4, MD5, MD6
/// - ADLER32
// TODO: why is SHA512 missing? Also SHA256 vs SHA-384
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "String", into = "&str")]
#[cfg_attr(
    feature = "python",
    pyclass(eq, eq_int, from_py_object),
    expect(clippy::unsafe_derive_deserialize)
)]
pub enum KerMlChecksumAlg {
    /// No checksum. Non-standard, must not be used in published
    /// versions of a project.
    /// Intended to be used in development to note that a file is
    /// included in the project without needing to recalculate
    /// checksum on every change.
    None,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha3_256,
    Sha3_384,
    Sha3_512,
    Blake2b256,
    Blake2b384,
    Blake2b512,
    Blake3,
    Md2,
    Md4,
    Md5,
    Md6,
    Adler32,
}

#[derive(Debug, Error)]
#[error("failed to parse checksum algorithm")]
pub struct AlgParseError;

impl TryFrom<String> for KerMlChecksumAlg {
    type Error = AlgParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl TryFrom<&str> for KerMlChecksumAlg {
    type Error = AlgParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        use KerMlChecksumAlg::*;
        let val = match value {
            "NONE" => None,

            "SHA1" => Sha1,
            "SHA224" => Sha224,
            "SHA256" => Sha256,
            "SHA-384" => Sha384,

            "SHA3-256" => Sha3_256,
            "SHA3-384" => Sha3_384,
            "SHA3-512" => Sha3_512,

            "BLAKE2b-256" => Blake2b256,
            "BLAKE2b-384" => Blake2b384,
            "BLAKE2b-512" => Blake2b512,
            "BLAKE3" => Blake3,

            "MD2" => Md2,
            "MD4" => Md4,
            "MD5" => Md5,
            "MD6" => Md6,

            "ADLER32" => Adler32,

            _ => return Err(AlgParseError),
        };
        Ok(val)
    }
}

impl From<KerMlChecksumAlg> for String {
    fn from(val: KerMlChecksumAlg) -> Self {
        let val: &str = val.into();
        val.to_owned()
    }
}

impl From<KerMlChecksumAlg> for &'static str {
    fn from(val: KerMlChecksumAlg) -> Self {
        use KerMlChecksumAlg::*;
        match val {
            None => "NONE",

            Sha1 => "SHA1",
            Sha224 => "SHA224",
            Sha256 => "SHA256",
            Sha384 => "SHA-384",

            Sha3_256 => "SHA3-256",
            Sha3_384 => "SHA3-384",
            Sha3_512 => "SHA3-512",

            Blake2b256 => "BLAKE2b-256",
            Blake2b384 => "BLAKE2b-384",
            Blake2b512 => "BLAKE2b-512",
            Blake3 => "BLAKE3",

            Md2 => "MD2",
            Md4 => "MD4",
            Md5 => "MD5",
            Md6 => "MD6",

            Adler32 => "ADLER32",
        }
    }
}

impl Display for KerMlChecksumAlg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s: &str = (*self).into();
        f.write_str(s)
    }
}

impl KerMlChecksumAlg {
    /// How long the hex-encoded checksum is for a given algorithm
    /// Formula: `checksum_len_bits / 4`, since each hex char is 4 bits
    pub fn expected_hex_len(&self) -> Option<u8> {
        use KerMlChecksumAlg::*;
        let len = match self {
            None => return Option::None,
            Sha1 => 40,
            Sha224 => 56,
            Sha256 | Sha3_256 | Blake2b256 | Blake3 => 64,
            Sha384 | Sha3_384 | Blake2b384 => 96,
            Sha3_512 | Blake2b512 => 128,
            Md2 | Md4 | Md5 => 32,
            // MD6 is variable length. TODO(spec): the
            // digest length must be somehow specified
            // Maybe specify as default MD6-256
            Md6 => return Option::None,
            Adler32 => 8,
        };
        Some(len)
    }
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(
    feature = "python",
    derive(FromPyObject, IntoPyObject),
    pyo3(from_item_all)
)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectChecksum {
    // TODO: use Vec<u8> or Box<[u8]> and store raw hash bytes
    pub value: String,
    pub algorithm: KerMlChecksumAlg,
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(
    feature = "python",
    derive(FromPyObject, IntoPyObject),
    pyo3(from_item_all)
)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectChecksumRaw {
    pub value: String,
    pub algorithm: String,
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(
    feature = "python",
    derive(FromPyObject, IntoPyObject),
    pyo3(from_item_all)
)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectMetadataG<Iri, Path: Eq + Hash, DateTime, IPC> {
    pub index: IndexMap<String, Path>,

    pub created: DateTime,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub metamodel: Option<Iri>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub includes_derived: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub includes_implied: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<IndexMap<Path, IPC>>,
}

pub type InterchangeProjectMetadataRaw =
    InterchangeProjectMetadataG<String, String, String, InterchangeProjectChecksumRaw>;
pub type InterchangeProjectMetadata = InterchangeProjectMetadataG<
    fluent_uri::Iri<String>,
    Utf8UnixPathBuf,
    jiff::Timestamp,
    InterchangeProjectChecksum,
>;

/// Canonical RFC 3339 serialization of the `created` timestamp. All code
/// that writes `InterchangeProjectMetadataRaw::created` goes through this
/// helper so the format stays consistent across producers (default
/// constructors, conversions, test fixtures) — two documents with the
/// same instant serialize byte-for-byte the same.
pub fn format_created(value: &jiff::Timestamp) -> String {
    format!("{value:.0}")
}

/// Shorthand for `format_created(&jiff::Timestamp::now())`, the most common
/// call site (default constructors and test fixtures all want "now").
pub fn format_created_now() -> String {
    format_created(&jiff::Timestamp::now())
}

impl From<InterchangeProjectMetadata> for InterchangeProjectMetadataRaw {
    fn from(value: InterchangeProjectMetadata) -> Self {
        InterchangeProjectMetadataRaw {
            index: value
                .index
                .into_iter()
                .map(|(k, v)| (k, v.into_string()))
                .collect(),
            created: format_created(&value.created),
            metamodel: value.metamodel.map(fluent_uri::Iri::into_string),
            includes_derived: value.includes_derived,
            includes_implied: value.includes_implied,
            checksum: value.checksum.map(|m| {
                m.into_iter()
                    .map(|(k, v)| {
                        (
                            k.into_string(),
                            InterchangeProjectChecksumRaw {
                                value: v.value,
                                algorithm: v.algorithm.to_string(),
                            },
                        )
                    })
                    .collect()
            }),
        }
    }
}

#[derive(Error, Debug)]
pub enum InterchangeProjectValidationError {
    #[error("invalid website (`website` field in `.project.json`) `{0}`")]
    InvalidWebsite(String, #[source] fluent_uri::ParseError),
    #[error("invalid usage resource `{0}`")]
    InvalidUsageResource(String, #[source] fluent_uri::ParseError),
    #[error("invalid metamodel (`metamodel` field in `.meta.json`) `{0}`")]
    InvalidMetamodel(String, #[source] fluent_uri::ParseError),
    #[error("project has an invalid Semantic Version `{0}`")]
    InvalidProjectVersion(Box<str>, #[source] semver::Error),
    #[error("project has an invalid name `{0}`")]
    InvalidProjectName(Box<str>, #[source] ProjectFieldError),
    #[error("project has an invalid publisher `{0}`")]
    InvalidProjectPublisher(Box<str>, #[source] ProjectFieldError),
    // spdx::ParseError formatting requires placing the error (which already
    // contains the original expression) as a first thing on a new line, so
    // do the whole formatting here
    #[error(
        "project has an invalid license (must be a valid SPDX license expression;\n\
        {LICENSE_EXPRESSION_HELP}):\n\
        {0}"
    )]
    InvalidProjectLicense(spdx::ParseError),
    #[error(
        "failed to parse version constraint `{constraint}` of usage\n\
        `{resource}` as a Semantic Version constraint"
    )]
    InvalidUsageVersionConstraint {
        resource: String,
        constraint: String,
        source: semver::Error,
    },
    #[error("exported symbol index (`index` field in `.meta.json`) references an invalid path")]
    InvalidPathInIndex(#[source] RelativeUnixPathError),
    #[error("source file checksum (`checksum` field in `.meta.json`) references an invalid path")]
    InvalidPathInChecksum(#[source] RelativeUnixPathError),
    #[error("path usage for `{publisher}`/`{name}` references an invalid path")]
    InvalidUsagePath {
        publisher: String,
        name: String,
        source: RelativeUnixPathError,
    },
    #[error("path usage for `{publisher}`/`{name}` has an invalid publisher")]
    InvalidUsagePublisher {
        publisher: String,
        name: String,
        source: ProjectFieldError,
    },
    #[error("path usage for `{publisher}`/`{name}` has an invalid name")]
    InvalidUsageName {
        publisher: String,
        name: String,
        source: ProjectFieldError,
    },
    #[error(
        "index usage `{publisher}/{name}` has an invalid publisher `{publisher}` \
         (3-50 ASCII alphanumeric chars, with single ` ` or `-` separators between words)"
    )]
    InvalidIndexUsagePublisher { publisher: String, name: String },
    #[error(
        "index usage `{publisher}/{name}` has an invalid name `{name}` \
         (3-50 ASCII alphanumeric chars, with single ` `, `-`, or `.` separators between words)"
    )]
    InvalidIndexUsageName { publisher: String, name: String },
    #[error(
        "failed to parse version constraint `{constraint}` of index usage\n\
        `{publisher}/{name}` as a Semantic Version constraint"
    )]
    InvalidIndexUsageVersionConstraint {
        publisher: String,
        name: String,
        constraint: String,
        source: semver::Error,
    },
    #[error("failed to parse `{0}` as RFC3339 datetime: {1}")]
    InvalidCreatedTime(Box<str>, jiff::Error),
    #[error(
        "invalid file checksum algorithm `{0}`, expected one of:\n\
        SHA1, SHA224, SHA256, SHA-384, SHA3-256, SHA3-384, SHA3-512\n\
        BLAKE2b-256, BLAKE2b-384, BLAKE2b-512, BLAKE3\n\
        MD2, MD4, MD5, MD6, ADLER32"
    )]
    InvalidChecksumAlg(Box<str>),
    #[error(
        "invalid hex checksum length for {algorithm}: expected {expected} char(s), got {got} char(s)"
    )]
    IncorrectChecksumLen {
        algorithm: KerMlChecksumAlg,
        expected: u8,
        got: usize,
    },
    #[error("checksum `{cksum}`\ncontains invalid symbols (only `A-Fa-f0-9` are allowed)")]
    NonHexChecksumChars { cksum: Box<str> },
    #[error("malformed `pkg:sysand` IRI `{iri}`")]
    MalformedUsageSysandPurl {
        iri: String,
        #[source]
        source: crate::purl::SysandPurlError,
    },
    #[error(
        "usage `{0}` is an identifier internal to Sysand; \
        use the project by its publisher and name instead"
    )]
    UrnSysandUsage(String),
}

impl Default for InterchangeProjectMetadataRaw {
    fn default() -> Self {
        InterchangeProjectMetadataRaw {
            index: IndexMap::default(),
            created: format_created_now(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }
    }
}

impl InterchangeProjectMetadataRaw {
    /// Caller is responsible for identifying the project when reporting the error.
    /// Check that `self` is valid according to KerML 1.0 spec. No additional checks
    /// are performed.
    pub fn validate(
        &self,
    ) -> Result<InterchangeProjectMetadata, InterchangeProjectValidationError> {
        let mut index = IndexMap::with_capacity(self.index.len());
        // Spec does not require any specific relationship between `index`
        // and `checksum` files
        for (symbol, path) in &self.index {
            let path = parse_relative_unix_path(path, RelativePathKind::SubFile)
                .map_err(InterchangeProjectValidationError::InvalidPathInIndex)?;
            index.insert(symbol.to_owned(), path.to_owned());
        }
        let checksum = if let Some(checksum) = &self.checksum {
            let mut res = IndexMap::with_capacity(checksum.len());
            for (k, v) in checksum {
                let k = parse_relative_unix_path(k, RelativePathKind::SubFile)
                    .map_err(InterchangeProjectValidationError::InvalidPathInChecksum)?
                    .to_path_buf();
                let algorithm: KerMlChecksumAlg =
                    v.algorithm.as_str().try_into().map_err(|_empty_err| {
                        InterchangeProjectValidationError::InvalidChecksumAlg(
                            v.algorithm.as_str().into(),
                        )
                    })?;
                let value = {
                    if let Some(expected_len) = algorithm.expected_hex_len() {
                        if v.value.len() != expected_len as usize {
                            return Err(InterchangeProjectValidationError::IncorrectChecksumLen {
                                algorithm,
                                expected: expected_len,
                                got: v.value.len(),
                            });
                        }
                        if !v.value.bytes().all(|c| c.is_ascii_hexdigit()) {
                            return Err(InterchangeProjectValidationError::NonHexChecksumChars {
                                cksum: v.value.as_str().into(),
                            });
                        }
                    }
                    v.value.clone()
                };
                res.insert(k, InterchangeProjectChecksum { value, algorithm });
            }

            Some(res)
        } else {
            None
        };

        let metamodel = if let Some(m) = &self.metamodel {
            if !KNOWN_METAMODELS.contains(&m.as_str()) {
                log::warn!("project uses an unknown metamodel `{m}`");
            }
            match fluent_uri::Iri::parse(m.to_owned()) {
                Ok(i) => Some(i),
                Err((e, val)) => {
                    return Err(InterchangeProjectValidationError::InvalidMetamodel(val, e));
                }
            }
        } else {
            None
        };

        Ok(InterchangeProjectMetadata {
            index,
            // TODO: this is not strictly correct, as RFC3339 only partially overlaps with ISO8601
            created: self.created.parse::<jiff::Timestamp>().map_err(|e| {
                InterchangeProjectValidationError::InvalidCreatedTime(
                    self.created.as_str().into(),
                    e,
                )
            })?,
            metamodel,
            includes_derived: self.includes_derived,
            includes_implied: self.includes_implied,
            checksum,
        })
    }

    /// Get symbols recorded in `index` for file at `path`
    pub fn file_index_symbols<P: AsRef<str>>(&self, path: P) -> HashSet<String> {
        self.index
            .iter()
            .filter_map(|(k, v)| {
                if v == path.as_ref() {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    // TODO: Get rid of overwrite
    /// Adds a checksum to the metadata.
    ///
    /// Overwrites any present value if `overwrite`.
    ///
    /// Returns the old checksum value, if present
    pub fn add_checksum<P: AsRef<Utf8UnixPath>, T: AsRef<str>>(
        &mut self,
        path: P,
        algorithm: KerMlChecksumAlg,
        value: T,
        overwrite: bool,
    ) -> Option<InterchangeProjectChecksumRaw> {
        let checksum = self.checksum.get_or_insert_with(IndexMap::default);

        match checksum.entry(path.as_ref().to_string()) {
            indexmap::map::Entry::Occupied(mut occupied_entry) => Some(if overwrite {
                occupied_entry.insert(InterchangeProjectChecksumRaw {
                    value: value.as_ref().to_owned(),
                    algorithm: algorithm.to_string(),
                })
            } else {
                occupied_entry.get().clone()
            }),
            indexmap::map::Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(InterchangeProjectChecksumRaw {
                    value: value.as_ref().to_owned(),
                    algorithm: algorithm.to_string(),
                });

                None
            }
        }
    }

    pub fn remove_checksum<P: AsRef<Utf8UnixPath>>(
        &mut self,
        path: &P,
    ) -> Option<InterchangeProjectChecksumRaw> {
        if let Some(checksum) = self.checksum.as_mut() {
            checksum.shift_remove(path.as_ref().as_str())
        } else {
            None
        }
    }

    pub fn remove_index<P: AsRef<Utf8UnixPath>>(&mut self, path: &P) -> Vec<String> {
        let remove_path = path.as_ref().as_str();

        self.index
            .extract_if(.., |_, v| v == remove_path)
            .map(|x| x.0)
            .collect()
    }
}

impl<Iri, Path: Eq + Hash + Clone, DateTime, IPC>
    InterchangeProjectMetadataG<Iri, Path, DateTime, IPC>
{
    pub fn minimal(created: DateTime) -> Self {
        Self {
            index: IndexMap::default(),
            created,
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }
    }

    pub fn source_paths(&self, include_index: bool) -> HashSet<Path> {
        let mut result: HashSet<Path> = HashSet::new();

        // TODO: Should these be normalised?
        if let Some(checksum) = &self.checksum {
            result.extend(
                checksum.keys().cloned(), //.map(|s| Utf8UnixPath::new(&s).to_path_buf()),
            );
        }

        if include_index {
            result.extend(
                self.index.values().cloned(), //.map(|s| Utf8UnixPath::new(&s).to_path_buf()),
            );
        }

        result
    }
}

pub type ProjectHash = Array<u8, typenum::U32>;

fn project_hash_str<S: AsRef<str>, T: AsRef<str>>(info: S, meta: T) -> ProjectHash {
    use digest::Digest as _;
    use sha2::Sha256;
    let mut hasher = Sha256::new();

    hasher.update(info.as_ref().as_bytes());
    hasher.update(meta.as_ref().as_bytes());

    hasher.finalize()
}

/// Use `project_hash_hex` where possible
pub fn project_hash_raw(
    info: &InterchangeProjectInfoRaw,
    meta: &InterchangeProjectMetadataRaw,
) -> ProjectHash {
    project_hash_str(
        serde_json::to_string(&info).expect("unexpected failure to serialise JSON"),
        serde_json::to_string(&meta).expect("unexpected failure to serialise JSON"),
    )
}

pub fn project_hash_hex(
    info: &InterchangeProjectInfoRaw,
    meta: &InterchangeProjectMetadataRaw,
) -> String {
    lowercase_hex(project_hash_raw(info, meta))
}

// Impose basic requirements on publisher/name

/// Reason a project publisher or name is invalid
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{kind} {reason}")]
pub struct ProjectFieldError {
    /// `publisher` or `name`
    kind: &'static str,
    reason: ProjectFieldErrorReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectFieldErrorReason {
    #[error("cannot be empty")]
    Empty,
    #[error("cannot be longer than {PROJECT_FIELD_MAX_LEN} bytes, but is {0} bytes long")]
    TooLong(usize),
    #[error("cannot contain `{0}`")]
    Reserved(char),
    #[error(
        "cannot contain {}; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
        describe_char(*.0)
    )]
    Disallowed(char),
    #[error("cannot start with {}", describe_char(*.0))]
    Start(char),
    #[error("cannot end with {}", describe_char(*.0))]
    End(char),
    #[error("must contain at least one letter or digit")]
    NoAlphanumeric,
    /// Valid as given, but not once normalized. As separators (including
    /// e.g. a fullwidth `／`) are folded into `-`, this only happens when
    /// compatibility decomposition makes it too long, e.g. U+0F77 expands
    /// into three characters
    #[error("normalizes to `{}`, which {}", .0.normalized, .0.reason)]
    Normalized(Box<NormalizedFieldError>),
}

/// Why a publisher or name is not valid once normalized, see
/// [`ProjectFieldErrorReason::Normalized`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedFieldError {
    pub normalized: String,
    pub reason: ProjectFieldErrorReason,
}

/// Describe `c` for an error message, also when it is invisible
fn describe_char(c: char) -> String {
    let escaped = c.escape_debug().to_string();
    if c == '`' {
        // Would be confusing inside backticks
        "U+0060 (backtick)".to_owned()
    } else if c.is_ascii_graphic() || c == ' ' {
        format!("`{c}`")
    } else if c.is_ascii() {
        // Invisible/control
        format!("`{escaped}`")
    } else if escaped.len() == c.len_utf8() {
        // Visible non-ASCII
        format!("`{c}` (U+{:04X})", u32::from(c))
    } else {
        // Invisible non-ASCII
        format!("U+{:04X}", u32::from(c))
    }
}

/// Maximum length of publisher/name in bytes. No specific reason,
/// but using longer names is probably a user error
const PROJECT_FIELD_MAX_LEN: usize = 300;

/// Whether `c` is allowed anywhere in publisher/name, ignoring positional
/// restrictions
fn is_project_field_char(c: char) -> bool {
    if c.is_ascii() {
        // Identifier chars are `a-zA-Z0-9_`
        XID_CONTINUE.contains(c) || PROJECT_FIELD_ASCII_PUNCTUATION.contains(&c)
    } else {
        // Allow all non-ASCII punctuation. Ignorable chars are excluded
        // as they are invisible
        (XID_CONTINUE.contains(c)
            || GeneralCategoryGroup::Punctuation.contains(GENERAL_CATEGORY.get(c)))
            && !IGNORABLE.contains(c)
    }
}

/// Validate a publisher or name (`kind`). Allowed characters are those of
/// Unicode identifiers (UAX #31 `XID_Continue`), except default ignorable
/// ones (invisible), plus punctuation (see [`is_project_field_char`])
fn validate_project_field(s: &str, kind: &'static str) -> Result<(), ProjectFieldError> {
    use ProjectFieldErrorReason as R;
    let err = |reason| Err(ProjectFieldError { kind, reason });
    let Some(first) = s.chars().next() else {
        return err(R::Empty);
    };
    let last = s.chars().next_back().unwrap();
    if s.len() > PROJECT_FIELD_MAX_LEN {
        return err(R::TooLong(s.len()));
    }
    let mut has_alphanumeric = false;
    for c in s.chars() {
        has_alphanumeric |= c.is_alphanumeric();
        if matches!(c, '/' | ':' | '<' | '>') {
            // Never allowed, regardless of the rules below:
            // `/`: allows unambiguously using `publisher/name` notation
            // `:`: not strictly necessary, but prevents using an IRI, which
            // could be confusing
            // `<`, `>`: useful for possible missing-publisher replacements
            // (e.g. "<none>")
            return err(R::Reserved(c));
        }
        if !is_project_field_char(c) {
            // Invisible or otherwise problematic chars
            return err(R::Disallowed(c));
        }
    }
    if first.is_whitespace() {
        return err(R::Start(first));
    }
    if last.is_whitespace() {
        return err(R::End(last));
    }
    if !has_alphanumeric {
        return err(R::NoAlphanumeric);
    }
    Ok(())
}

/// Validate a publisher or name (`kind`), and its normalized form, which
/// must be valid too. Returns the normalized form
fn parse_project_field(s: &str, kind: &'static str) -> Result<String, ProjectFieldError> {
    validate_project_field(s, kind)?;
    // TODO: add a fast path for index-compliant spellings (the common case):
    // normalize them with `normalize_index_field`, which is equivalent for
    // them, and skip re-validating the result, which is then always valid.
    // Then `normalize_typed_publisher`/`normalize_typed_name` and
    // `Identifier::make_identifier_iri` would no longer need to try the
    // index types first to avoid the full Unicode normalization
    let normalized = normalize(s);
    validate_project_field(&normalized, kind).map_err(|e| ProjectFieldError {
        kind,
        reason: ProjectFieldErrorReason::Normalized(Box::new(NormalizedFieldError {
            normalized: normalized.clone(),
            reason: e.reason,
        })),
    })?;
    Ok(normalized)
}

/// The publisher of a project, as spelled and normalized
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectPublisher {
    spelling: String,
    normalized: String,
}

impl ProjectPublisher {
    pub fn parse(publisher: String) -> Result<Self, (String, ProjectFieldError)> {
        match parse_project_field(&publisher, "publisher") {
            Ok(normalized) => Ok(Self {
                spelling: publisher,
                normalized,
            }),
            Err(e) => Err((publisher, e)),
        }
    }

    /// The normalized publisher
    pub fn normalized(&self) -> &str {
        &self.normalized
    }

    /// The normalized publisher
    pub fn into_normalized(self) -> String {
        self.normalized
    }

    pub fn as_str(&self) -> &str {
        &self.spelling
    }

    pub fn into_string(self) -> String {
        self.spelling
    }
}

/// The name of a project, as spelled and normalized
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectName {
    spelling: String,
    normalized: String,
}

impl ProjectName {
    pub fn parse(name: String) -> Result<Self, (String, ProjectFieldError)> {
        match parse_project_field(&name, "name") {
            Ok(normalized) => Ok(Self {
                spelling: name,
                normalized,
            }),
            Err(e) => Err((name, e)),
        }
    }

    /// The normalized name
    pub fn normalized(&self) -> &str {
        &self.normalized
    }

    /// The normalized name
    pub fn into_normalized(self) -> String {
        self.normalized
    }

    pub fn as_str(&self) -> &str {
        &self.spelling
    }

    pub fn into_string(self) -> String {
        self.spelling
    }
}

/// Why a publisher or name cannot spell a project in an index usage (or an
/// index), see [`IndexPublisher`] and [`IndexName`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IndexFieldError {
    #[error(
        "publisher must be 3-50 ASCII alphanumeric chars, with single ` ` or `-` separators \
         between words"
    )]
    Publisher,
    #[error(
        "name must be 3-50 ASCII alphanumeric chars, with single ` `, `-`, or `.` separators \
         between words"
    )]
    Name,
}

/// The publisher of a project as an index usage, and an index, spell it:
/// valid, once normalized (see [`Self::normalized`]), as the
/// publisher of a `pkg:sysand` PURL. Every such publisher is also a valid
/// [`ProjectPublisher`], but not the other way around
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct IndexPublisher(String);

impl IndexPublisher {
    pub fn parse(publisher: String) -> Result<Self, (String, IndexFieldError)> {
        if crate::purl::is_valid_unnormalized_publisher(&publisher) {
            Ok(Self(publisher))
        } else {
            Err((publisher, IndexFieldError::Publisher))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// The publisher as the `pkg:sysand` PURL holds it
    pub fn normalized(&self) -> String {
        normalize_index_field(&self.0)
    }

    /// Whether the publisher is spelled as [`Self::normalized`] makes it
    pub fn is_normalized(&self) -> bool {
        is_normalized_index_field(&self.0)
    }
}

/// The name of a project as an index usage, and an index, spell it: valid,
/// once normalized (see [`Self::normalized`]), as the name of a
/// `pkg:sysand` PURL. Every such name is also a valid [`ProjectName`], but
/// not the other way around
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct IndexName(String);

impl IndexName {
    pub fn parse(name: String) -> Result<Self, (String, IndexFieldError)> {
        if crate::purl::is_valid_unnormalized_name(&name) {
            Ok(Self(name))
        } else {
            Err((name, IndexFieldError::Name))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// The name as the `pkg:sysand` PURL holds it
    pub fn normalized(&self) -> String {
        normalize_index_field(&self.0)
    }

    /// Whether the name is spelled as [`Self::normalized`] makes it
    pub fn is_normalized(&self) -> bool {
        is_normalized_index_field(&self.0)
    }
}

/// How the publisher of a typed usage of any kind normalizes: as
/// [`IndexPublisher::normalized`] if it is spelled as an index usage can
/// spell it, otherwise as [`ProjectPublisher::normalized`]. A publisher that
/// is not even a valid project publisher is kept as it is
pub(crate) fn normalize_typed_publisher(publisher: &str) -> String {
    match IndexPublisher::parse(publisher.to_owned()) {
        Ok(publisher) => publisher.normalized(),
        Err((publisher, _)) => match ProjectPublisher::parse(publisher) {
            Ok(publisher) => publisher.into_normalized(),
            Err((publisher, _)) => publisher,
        },
    }
}

/// How the name of a typed usage of any kind normalizes, as
/// [`normalize_typed_publisher`] does for the publisher
pub(crate) fn normalize_typed_name(name: &str) -> String {
    match IndexName::parse(name.to_owned()) {
        Ok(name) => name.normalized(),
        Err((name, _)) => match ProjectName::parse(name) {
            Ok(name) => name.into_normalized(),
            Err((name, _)) => name,
        },
    }
}

macro_rules! index_field_impls {
    ($ty:ty) => {
        impl TryFrom<String> for $ty {
            type Error = IndexFieldError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value).map_err(|(_, e)| e)
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.0
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

index_field_impls!(IndexPublisher);
index_field_impls!(IndexName);

impl TryFrom<String> for ProjectPublisher {
    type Error = ProjectFieldError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value).map_err(|(_, e)| e)
    }
}

impl From<ProjectPublisher> for String {
    fn from(value: ProjectPublisher) -> Self {
        value.spelling
    }
}

impl AsRef<str> for ProjectPublisher {
    fn as_ref(&self) -> &str {
        &self.spelling
    }
}

impl Display for ProjectPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.spelling)
    }
}

impl TryFrom<String> for ProjectName {
    type Error = ProjectFieldError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value).map_err(|(_, e)| e)
    }
}

impl From<ProjectName> for String {
    fn from(value: ProjectName) -> Self {
        value.spelling
    }
}

impl AsRef<str> for ProjectName {
    fn as_ref(&self) -> &str {
        &self.spelling
    }
}

impl Display for ProjectName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.spelling)
    }
}

/// Normalize a non-index field. For index-compliant strings this
/// is equivalent to [`normalize_index_field`] (and must remain so),
/// but it's very expensive and so the simplified version remains
///
/// After NFKC_Casefold, each run of separators (see [`is_word_char`])
/// becomes a single [`PURL_SEPARATOR`], except a run of a single
/// [`PURL_NAME_SEPARATOR`], which is kept, and runs at either end are
/// dropped. The result only contains word characters, [`PURL_SEPARATOR`]
/// and [`PURL_NAME_SEPARATOR`]. An index-compliant string only has single
/// separators between words, which stay as [`normalize_index_field`] makes
/// them
fn normalize(input: &str) -> String {
    // Implements Unicode D147 (Identifier normalization for comparison),
    // aka NFKC_Casefold(NFD(X)), which icu4x does not provide.
    // Since IGNORABLE codepoints are not present, this simplifies to
    // NFKC(CaseFold(NFKD(X)))
    let decomp = NFKD_NORMALIZER.normalize(input);
    let folded = CASE_MAPPER.fold_string(&decomp);
    let composed = NFKC_NORMALIZER.normalize(&folded);

    let mut normalized = String::with_capacity(composed.len());
    // Length of the current run of separators, and whether it starts with
    // `PURL_NAME_SEPARATOR`
    let mut run = 0usize;
    let mut run_starts_with_dot = false;
    for c in composed.chars() {
        if is_word_char(c) {
            // A run at the start is dropped
            if run > 0 && !normalized.is_empty() {
                normalized.push(char::from(if run == 1 && run_starts_with_dot {
                    PURL_NAME_SEPARATOR
                } else {
                    PURL_SEPARATOR
                }));
            }
            run = 0;
            normalized.push(c);
        } else {
            if run == 0 {
                run_starts_with_dot = c == char::from(PURL_NAME_SEPARATOR);
            }
            run += 1;
        }
    }
    // A run at the end is dropped by never being pushed
    normalized
}

/// Whether `c` is part of a word of a normalized publisher or name: an
/// identifier character (`XID_Continue`) other than punctuation, such as
/// `_`. Everything else separates words
fn is_word_char(c: char) -> bool {
    XID_CONTINUE.contains(c) && !GeneralCategoryGroup::Punctuation.contains(GENERAL_CATEGORY.get(c))
}

/// Lowercases ASCII and replaces spaces with hyphens. Only for the
/// normalization of [`IndexPublisher`] and [`IndexName`]
fn normalize_index_field(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == char::from(UNNORMALIZED_PURL_SEPARATOR) {
                char::from(PURL_SEPARATOR)
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

/// Whether `s` is already what [`normalize_index_field`] makes of it: it has
/// no ASCII uppercase letter and no space
fn is_normalized_index_field(s: &str) -> bool {
    !s.bytes()
        .any(|b| b.is_ascii_uppercase() || b == UNNORMALIZED_PURL_SEPARATOR)
}

#[cfg(test)]
#[path = "./model_tests.rs"]
mod tests;
