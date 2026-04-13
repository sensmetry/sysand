// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{clone::Clone, collections::HashSet, fmt::Display, hash::Hash};

#[allow(deprecated)] // will change when `sha2` 0.11 is released
use digest::{generic_array::GenericArray, typenum};
use indexmap::IndexMap;
#[cfg(feature = "python")]
use pyo3::{FromPyObject, IntoPyObject, pyclass};
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::{lock::Usage, project::utils::make_identifier_iri};

// pub struct RawIri(String);
// pub struct ParsedIri(fluent_uri::Iri<String>);
// pub struct NormalisedIri(fluent_uri::Iri<String>);

pub const KNOWN_METAMODELS: [&str; 2] = [
    "https://www.omg.org/spec/SysML/20250201",
    "https://www.omg.org/spec/KerML/20250201",
];

// #[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Hash, Debug)]
// #[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
// #[serde(rename_all = "camelCase")]
// pub struct InterchangeProjectUsageG<Iri, VersionReq> {
//     pub resource: Iri, // TODO: We should have a fallback for invalid IRIs
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub version_constraint: Option<VersionReq>, // TODO: We should have a fallback for invalid semvers
// }
// pub type InterchangeProjectUsageRaw = InterchangeProjectUsageG<String, String>;
// pub type InterchangeProjectUsage =
//     InterchangeProjectUsageG<fluent_uri::Iri<String>, semver::VersionReq>;

// impl InterchangeProjectUsageRaw {
//     pub fn validate(&self) -> Result<InterchangeProjectUsage, InterchangeProjectValidationError> {
//         Ok(InterchangeProjectUsage {
//             resource: fluent_uri::Iri::parse(self.resource.clone())
//                 .map_err(|(e, val)| InterchangeProjectValidationError::IriParse(val, e))?,

//             version_constraint: self
//                 .version_constraint
//                 .as_ref()
//                 .map(|c| {
//                     semver::VersionReq::parse(c).map_err(|e| {
//                         InterchangeProjectValidationError::SemVerConstraintParse(c.to_owned(), e)
//                     })
//                 })
//                 .transpose()?,
//         })
//     }
// }

// impl From<InterchangeProjectUsage> for InterchangeProjectUsageRaw {
//     fn from(value: InterchangeProjectUsage) -> InterchangeProjectUsageRaw {
//         InterchangeProjectUsageRaw {
//             resource: value.resource.to_string(),
//             version_constraint: value.version_constraint.map(|x| x.to_string()),
//         }
//     }
// }

// impl From<InterchangeProjectUsageG<fluent_uri::Iri<String>, semver::VersionReq>>
//     for InterchangeProjectUsageG<String, semver::VersionReq>
// {
//     fn from(value: InterchangeProjectUsageG<fluent_uri::Iri<String>, semver::VersionReq>) -> Self {
//         InterchangeProjectUsageG {
//             resource: value.resource.to_string(),
//             version_constraint: value.version_constraint,
//         }
//     }
// }

// impl TryFrom<InterchangeProjectUsageRaw> for InterchangeProjectUsage {
//     type Error = InterchangeProjectValidationError;

//     fn try_from(value: InterchangeProjectUsageRaw) -> Result<InterchangeProjectUsage, Self::Error> {
//         value.validate()
//     }
// }

// TODO: maybe make this generic over AsRef<str>?
#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Hash, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase", untagged)]
pub enum GitId {
    Rev(String),
    Tag(String),
    Branch(String),
}

impl Display for GitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitId::Rev(r) => write!(f, "commit `{r}`"),
            GitId::Tag(t) => write!(f, "tag `{t}`"),
            GitId::Branch(b) => write!(f, "branch `{b}`"),
        }
    }
}

/// Usage of a project. Legacy (KerML 1.0) usage is always `Resource`,
/// regardless of its actual type.
/// `Publisher` and `Name` can be inferred from downloaded project, so there is
/// no need for user to provide this info in most cases. This is required
/// only if user disables lock/sync when adding a usage, or if non-root
/// project of git repo is to be used.
#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Hash, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase", untagged)]
pub enum InterchangeProjectUsageG<Iri, VersionReq, Path> {
    /// Legacy, from KerML 1.0 spec
    Resource {
        resource: Iri, // TODO: We should have a fallback for invalid IRIs
        #[serde(skip_serializing_if = "Option::is_none")]
        version_constraint: Option<VersionReq>, // TODO: We should have a fallback for invalid semvers
    },
    Url {
        url: Iri,
        publisher: String,
        name: String,
    },
    // TODO: assuming this is a relative Unix-style path
    // TODO: use proper types
    Path {
        path: Path,
        publisher: String,
        name: String,
    },
    Git {
        git: Iri,
        id: GitId,
        publisher: String,
        name: String,
    },
    Index {
        publisher: String,
        name: String,
        version_constraint: VersionReq,
    },
    // TODO: is this needed? We don't know what info might be needed for different APIs,
    // so it seems premature to include this here, as it would likely be useless
    // TODO: change the doc, it seemingly by mistake lists rev/tag/branch here
    // Api {
    //     server: Iri,
    //     publisher: String,
    //     name: String,
    //     project_id: u128, // UUID
    // },
}

pub type InterchangeProjectUsageRaw = InterchangeProjectUsageG<String, String, String>;
pub type InterchangeProjectUsage =
    InterchangeProjectUsageG<fluent_uri::Iri<String>, semver::VersionReq, Utf8UnixPathBuf>;

impl<Iri: Display, VersionReq: Display, Path: Display> Display
    for InterchangeProjectUsageG<Iri, VersionReq, Path>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InterchangeProjectUsageG::Resource {
                resource,
                version_constraint,
            } => {
                write!(f, "IRI `{resource}`")?;
                if let Some(vc) = version_constraint {
                    write!(f, " ({vc})")?;
                }
            }
            InterchangeProjectUsageG::Url {
                url,
                publisher,
                name,
            } => {
                write!(f, "`{publisher}/{name}` from URL `{url}`")?;
            }
            InterchangeProjectUsageG::Path {
                path,
                publisher,
                name,
            } => {
                write!(f, "`{publisher}/{name}` from path `{path}`")?;
            }
            InterchangeProjectUsageG::Git {
                git,
                id,
                publisher,
                name,
            } => {
                write!(f, "`{publisher}/{name}` from git repository `{git}`, {id}")?;
            }
            InterchangeProjectUsageG::Index {
                publisher,
                name,
                // TODO: version must be chosen at this point even if not provided by user
                version_constraint,
            } => {
                write!(f, "`{publisher}/{name}` ({version_constraint}) from index")?;
            }
        }
        Ok(())
    }
}

impl<Iri: Display, VersionReq, Path> InterchangeProjectUsageG<Iri, VersionReq, Path> {
    pub fn from_iri(iri: Iri) -> Self {
        InterchangeProjectUsageG::Resource {
            resource: iri,
            version_constraint: None,
        }
    }

    pub fn from_iri_version(iri: Iri, version: VersionReq) -> Self {
        InterchangeProjectUsageG::Resource {
            resource: iri,
            version_constraint: Some(version),
        }
    }

    /// Get the canonical IRI representing this usage. This IRI is not resolvable
    /// on its own.
    /// This is expensive, don't call repeatedly
    pub fn to_lock_usage(&self) -> Usage {
        match self {
            InterchangeProjectUsageG::Resource {
                resource,
                version_constraint: _,
            } => Usage::from(resource.to_string()),
            InterchangeProjectUsageG::Url {
                url: _,
                publisher,
                name,
            } => Usage::from(make_identifier_iri(publisher, name)),
            InterchangeProjectUsageG::Path {
                path: _,
                publisher,
                name,
            } => Usage::from(make_identifier_iri(publisher, name)),
            InterchangeProjectUsageG::Git {
                git: _,
                id: _,
                publisher,
                name,
            } => Usage::from(make_identifier_iri(publisher, name)),
            InterchangeProjectUsageG::Index {
                publisher,
                name,
                version_constraint: _,
            } => Usage::from(make_identifier_iri(publisher, name)),
        }
    }
}

impl InterchangeProjectUsage {
    /// Returns true if there is no constraint
    pub fn version_satisfies_req(&self, version: &Version) -> bool {
        match self {
            InterchangeProjectUsage::Resource {
                resource: _,
                version_constraint,
            } => {
                if let Some(vc) = version_constraint {
                    vc.matches(version)
                } else {
                    true
                }
            }
            InterchangeProjectUsage::Index {
                publisher: _,
                name: _,
                version_constraint,
            } => version_constraint.matches(version),
            InterchangeProjectUsage::Url { .. } => true,
            InterchangeProjectUsage::Path { .. } => true,
            InterchangeProjectUsage::Git { .. } => true,
        }
    }
}

impl InterchangeProjectUsageRaw {
    // TODO: consolidate to `try_from()`?
    pub fn validate(&self) -> Result<InterchangeProjectUsage, InterchangeProjectValidationError> {
        let res = match self {
            InterchangeProjectUsageG::Resource {
                resource,
                version_constraint,
            } => InterchangeProjectUsage::Resource {
                resource: fluent_uri::Iri::parse(resource.to_owned())
                    .map_err(|(e, val)| InterchangeProjectValidationError::IriParse(val, e))?,

                version_constraint: version_constraint
                    .as_ref()
                    .map(|c| {
                        semver::VersionReq::parse(c).map_err(|e| {
                            InterchangeProjectValidationError::SemVerConstraintParse(
                                c.to_owned(),
                                e,
                            )
                        })
                    })
                    .transpose()?,
            },
            InterchangeProjectUsageG::Url {
                url,
                publisher,
                name,
            } => InterchangeProjectUsage::Url {
                url: fluent_uri::Iri::parse(url.to_owned())
                    .map_err(|(e, val)| InterchangeProjectValidationError::IriParse(val, e))?,
                publisher: publisher.clone(),
                name: name.clone(),
            },
            InterchangeProjectUsageG::Path {
                path,
                publisher,
                name,
            } => InterchangeProjectUsage::Path {
                // TODO: check that this is a relative Unix path
                path: Utf8UnixPathBuf::from(path),
                publisher: publisher.clone(),
                name: name.clone(),
            },
            InterchangeProjectUsageG::Git {
                git,
                id,
                publisher,
                name,
            } => InterchangeProjectUsage::Git {
                git: fluent_uri::Iri::parse(git.to_owned())
                    .map_err(|(e, val)| InterchangeProjectValidationError::IriParse(val, e))?,
                id: id.clone(),
                // TODO: No restrictions for now
                publisher: publisher.to_owned(),
                name: name.to_owned(),
            },
            InterchangeProjectUsageG::Index {
                publisher,
                name,
                version_constraint,
            } => InterchangeProjectUsage::Index {
                publisher: publisher.to_owned(),
                name: name.to_owned(),
                version_constraint: semver::VersionReq::parse(version_constraint).map_err(|e| {
                    InterchangeProjectValidationError::SemVerConstraintParse(
                        version_constraint.to_owned(),
                        e,
                    )
                })?,
            },
        };

        Ok(res)
    }

    // /// Get the canonical IRI representing this usage. This IRI is not resolvable
    // /// on its own.
    // /// This is expensive, don't call repeatedly
    // pub fn to_lock_usage(&self) -> Usage {
    //     match self {
    //         InterchangeProjectUsageG::Resource {
    //             resource,
    //             version_constraint: _,
    //         } => Usage::from(resource.to_owned()),
    //         InterchangeProjectUsageG::Url {
    //             url: _,
    //             publisher,
    //             name,
    //         } => Usage::from(make_identifier_iri(publisher, name)),
    //         InterchangeProjectUsageG::Path {
    //             path: _,
    //             publisher,
    //             name,
    //         } => Usage::from(make_identifier_iri(publisher, name)),
    //         InterchangeProjectUsageG::Git {
    //             git: _,
    //             id: _,
    //             publisher,
    //             name,
    //         } => Usage::from(make_identifier_iri(publisher, name)),
    //         InterchangeProjectUsageG::Index {
    //             publisher,
    //             name,
    //             version_constraint: _,
    //         } => Usage::from(make_identifier_iri(publisher, name)),
    //     }
    // }
}

impl From<InterchangeProjectUsage> for InterchangeProjectUsageRaw {
    fn from(value: InterchangeProjectUsage) -> InterchangeProjectUsageRaw {
        match value {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint,
            } => InterchangeProjectUsageRaw::Resource {
                resource: resource.into_string(),
                version_constraint: version_constraint.map(|x| x.to_string()),
            },
            InterchangeProjectUsage::Url {
                url,
                publisher,
                name,
            } => InterchangeProjectUsageRaw::Url {
                url: url.into_string(),
                publisher,
                name,
            },
            InterchangeProjectUsage::Path {
                path,
                publisher,
                name,
            } => InterchangeProjectUsageRaw::Path {
                path: path.into_string(),
                publisher,
                name,
            },
            InterchangeProjectUsage::Git {
                git,
                id,
                publisher,
                name,
            } => InterchangeProjectUsageRaw::Git {
                git: git.into_string(),
                id,
                publisher,
                name,
            },
            InterchangeProjectUsage::Index {
                publisher,
                name,
                version_constraint,
            } => InterchangeProjectUsageRaw::Index {
                publisher,
                name,
                version_constraint: version_constraint.to_string(),
            },
        }
    }
}

// impl From<InterchangeProjectUsageG<fluent_uri::Iri<String>, semver::VersionReq>>
//     for InterchangeProjectUsageG<String, semver::VersionReq>
// {
//     fn from(value: InterchangeProjectUsageG<fluent_uri::Iri<String>, semver::VersionReq>) -> Self {
//         InterchangeProjectUsageG {
//             resource: value.resource.to_string(),
//             version_constraint: value.version_constraint,
//         }
//     }
// }

impl TryFrom<InterchangeProjectUsageRaw> for InterchangeProjectUsage {
    type Error = InterchangeProjectValidationError;

    fn try_from(value: InterchangeProjectUsageRaw) -> Result<InterchangeProjectUsage, Self::Error> {
        value.validate()
    }
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectInfoG<Iri, Version, VersionReq, Path> {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    pub version: Version, // TODO We should have a fallback for invalid semvers

    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub maintainer: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<Iri>, // TODO We should have a fallback for invalid IRIs

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub topic: Vec<String>,

    // pub usage: Vec<InterchangeProjectUsageG<Iri, VersionReq>>,
    pub usage: Vec<InterchangeProjectUsageG<Iri, VersionReq, Path>>,
}

pub type InterchangeProjectInfoRaw = InterchangeProjectInfoG<String, String, String, String>;
pub type InterchangeProjectInfo = InterchangeProjectInfoG<
    fluent_uri::Iri<String>,
    semver::Version,
    semver::VersionReq,
    Utf8UnixPathBuf,
>;

impl From<InterchangeProjectInfo> for InterchangeProjectInfoRaw {
    fn from(value: InterchangeProjectInfo) -> Self {
        InterchangeProjectInfoRaw {
            name: value.name,
            publisher: value.publisher,
            description: value.description,
            version: value.version.to_string(),
            license: value.license,
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

impl<Iri: PartialEq + Clone, Version, VersionReq: Clone, Path>
    InterchangeProjectInfoG<Iri, Version, VersionReq, Path>
{
    pub fn minimal(name: String, version: Version) -> Self {
        InterchangeProjectInfoG {
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
    // pub fn push_usage(&mut self, resource: Iri, version_requirement: Option<VersionReq>) {
    //     self.usage.push(InterchangeProjectUsageG {
    //         resource: resource,
    //         version_constraint: version_requirement,
    //     });
    // }

    /// Remove and return all occurrences of `resource` in project usages.
    /// Note that sysand will never add multiple usages of the same resource
    /// to the project, but it does tolerate such usages.
    // TODO: the spec does not say anything about this and should be clarified
    pub fn pop_usage(
        &mut self,
        resource: &Iri,
    ) -> Vec<InterchangeProjectUsageG<Iri, VersionReq, Path>> {
        self.usage
            .extract_if(.., |u| {
                if let InterchangeProjectUsageG::Resource { resource: r, .. } = u
                    && r == resource
                {
                    true
                } else {
                    false
                }
            })
            .collect()
    }

    /// Remove and return all usages matching `publisher`/`name`.
    /// Note that sysand will never add multiple usages of the same resource
    /// to the project, but it does tolerate such usages.
    // TODO: the spec does not say anything about this and should be clarified
    pub fn pop_usage_experimental(
        &mut self,
        publisher: impl AsRef<str>,
        name: impl AsRef<str>,
    ) -> Vec<InterchangeProjectUsageG<Iri, VersionReq, Path>> {
        let p = publisher.as_ref();
        let n = name.as_ref();
        self.usage
            .extract_if(.., |u| match u {
                // TODO: how to match here? Simplest would be to require the same info as for
                // adding, but that is way overkill and annoying to use. Otherwise we'd need
                // some sort of separate "matcher" type that allows wildcarding everything
                // apart from: any sort of IRI/URL, publisher+name.
                // Then how to allow providing version (constraint) and possibly other matchers?
                InterchangeProjectUsageG::Resource { .. } => false,
                InterchangeProjectUsageG::Url {
                    publisher, name, ..
                }
                | InterchangeProjectUsageG::Path {
                    publisher, name, ..
                }
                | InterchangeProjectUsageG::Git {
                    publisher, name, ..
                }
                | InterchangeProjectUsageG::Index {
                    publisher, name, ..
                } => publisher == p && name == n,
            })
            .collect()
    }
}

impl InterchangeProjectInfoRaw {
    pub fn validate(&self) -> Result<InterchangeProjectInfo, InterchangeProjectValidationError> {
        let mut usage = vec![];
        for a_usage in self.usage.iter() {
            usage.push(a_usage.to_owned().try_into()?);
        }

        Ok(InterchangeProjectInfo {
            name: self.name.clone(),
            publisher: self.publisher.clone(),
            description: self.description.clone(),
            version: semver::Version::parse(&self.version).map_err(|e| {
                InterchangeProjectValidationError::SemVerParse(self.version.as_str().into(), e)
            })?,
            license: self.license.clone(),
            maintainer: self.maintainer.clone(),
            website: self
                .website
                .clone()
                .map(fluent_uri::Iri::parse)
                .transpose()
                .map_err(|(e, val)| InterchangeProjectValidationError::IriParse(val, e))?,

            topic: self.topic.clone(),
            usage,
        })
    }
}

impl TryFrom<InterchangeProjectInfoRaw> for InterchangeProjectInfo {
    type Error = InterchangeProjectValidationError;

    fn try_from(value: InterchangeProjectInfoRaw) -> Result<Self, Self::Error> {
        value.validate()
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
#[cfg_attr(feature = "python", pyclass(eq, eq_int, from_py_object))]
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
        val.to_string()
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
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectChecksum {
    // TODO: use Vec<u8> or Box<[u8]> and store raw hash bytes
    pub value: String,
    pub algorithm: KerMlChecksumAlg,
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectChecksumRaw {
    pub value: String,
    pub algorithm: String,
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
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
    chrono::DateTime<chrono::Utc>,
    InterchangeProjectChecksum,
>;

impl From<InterchangeProjectMetadata> for InterchangeProjectMetadataRaw {
    fn from(value: InterchangeProjectMetadata) -> InterchangeProjectMetadataRaw {
        InterchangeProjectMetadataRaw {
            index: value
                .index
                .into_iter()
                .map(|(k, v)| (k, v.into_string()))
                .collect(),
            created: value
                .created
                .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            metamodel: value.metamodel.map(|iri| iri.into_string()),
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
    #[error("failed to parse `{0}` as IRI: {1}")]
    IriParse(String, fluent_uri::ParseError),
    #[error("failed to parse `{0}` as a Semantic Version: {1}")]
    SemVerParse(Box<str>, semver::Error),
    #[error("failed to parse `{0}` as a Semantic Version constraint: {1}")]
    SemVerConstraintParse(String, semver::Error),
    #[error("failed to parse `{0}` as RFC3339 datetime: {1}")]
    DatetimeParse(Box<str>, chrono::ParseError),
    #[error("file `{0}` is present in symbol index, but absent in file checksums")]
    MissingFileInChecksum(Box<str>),
    #[error(
        "invalid file checksum algorithm `{0}`, expected one of:\n\
        SHA1, SHA224, SHA256, SHA-384, SHA3-256, SHA3-384, SHA3-512\n\
        BLAKE2b-256, BLAKE2b-384, BLAKE2b-512, BLAKE3\n\
        MD2, MD4, MD5, MD6, ADLER32"
    )]
    ChecksumAlg(Box<str>),
    #[error(
        "invalid hex checksum length for {algorithm}: expected {expected} char(s), got {got} char(s)"
    )]
    ChecksumLen {
        algorithm: KerMlChecksumAlg,
        expected: u8,
        got: usize,
    },
    #[error("checksum `{cksum}`\ncontains invalid symbols (only `A-Fa-f0-9` are allowed)")]
    NonHexChecksumChars { cksum: Box<str> },
}

impl Default for InterchangeProjectMetadataRaw {
    fn default() -> Self {
        InterchangeProjectMetadataRaw {
            index: IndexMap::default(),
            created: chrono::Utc::now().to_rfc3339(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }
    }
}

impl InterchangeProjectMetadataRaw {
    pub fn validate(
        &self,
    ) -> Result<InterchangeProjectMetadata, InterchangeProjectValidationError> {
        let checksum = if let Some(checksum) = &self.checksum {
            // Checksum must include all the files mentioned in index,
            // but index may mention less files than checksum.
            for path in self.index.values() {
                if !checksum.contains_key(path) {
                    return Err(InterchangeProjectValidationError::MissingFileInChecksum(
                        path.as_str().into(),
                    ));
                }
            }

            let mut res = IndexMap::with_capacity(checksum.len());
            for (k, v) in checksum {
                let k = Utf8UnixPath::new(k).to_path_buf();
                let algorithm: KerMlChecksumAlg =
                    v.algorithm.as_str().try_into().map_err(|_| {
                        InterchangeProjectValidationError::ChecksumAlg(v.algorithm.as_str().into())
                    })?;
                let value = {
                    if let Some(expected_len) = algorithm.expected_hex_len() {
                        if v.value.len() != expected_len as usize {
                            return Err(InterchangeProjectValidationError::ChecksumLen {
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

        Ok(InterchangeProjectMetadata {
            index: self
                .index
                .iter()
                .map(|(k, v)| (k.to_owned(), Utf8UnixPath::new(v).to_path_buf()))
                .collect(),
            // TODO: this is not strictly correct, as RFC3339 only partially overlaps with ISO8601
            created: chrono::DateTime::parse_from_rfc3339(&self.created)
                .map_err(|e| {
                    InterchangeProjectValidationError::DatetimeParse(
                        self.created.as_str().into(),
                        e,
                    )
                })?
                .into(),
            metamodel: self
                .metamodel
                .clone()
                .map(|m| {
                    if !KNOWN_METAMODELS.contains(&m.as_str()) {
                        log::warn!("project uses an unknown metamodel `{}`", m);
                    }
                    fluent_uri::Iri::parse(m)
                })
                .transpose()
                .map_err(|(e, val)| InterchangeProjectValidationError::IriParse(val, e))?,
            includes_derived: self.includes_derived,
            includes_implied: self.includes_implied,
            checksum,
        })
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
                    value: value.as_ref().to_string(),
                    algorithm: algorithm.to_string(),
                })
            } else {
                occupied_entry.get().clone()
            }),
            indexmap::map::Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(InterchangeProjectChecksumRaw {
                    value: value.as_ref().to_string(),
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

    // pub fn remove_index_from<P: AsRef<Utf8UnixPath>>(&mut self, path: &P) {
    //     todo!()
    // }
}

impl TryFrom<InterchangeProjectMetadataRaw> for InterchangeProjectMetadata {
    type Error = InterchangeProjectValidationError;

    fn try_from(
        value: InterchangeProjectMetadataRaw,
    ) -> Result<InterchangeProjectMetadata, Self::Error> {
        value.validate()
    }
}

impl<Iri, Path: Eq + Hash + Clone, DateTime, IPC>
    InterchangeProjectMetadataG<Iri, Path, DateTime, IPC>
{
    pub fn minimal(created: DateTime) -> Self {
        InterchangeProjectMetadataG {
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

#[allow(deprecated)] // will change when `sha2` 0.11 is released
pub type ProjectHash = GenericArray<u8, typenum::U32>;

pub fn project_hash_str<S: AsRef<str>, T: AsRef<str>>(info: S, meta: T) -> ProjectHash {
    use digest::Digest;
    use sha2::Sha256;
    let mut hasher = Sha256::new();

    hasher.update(info.as_ref().as_bytes());
    hasher.update(meta.as_ref().as_bytes());

    hasher.finalize()
}

pub fn project_hash_raw(
    info: &InterchangeProjectInfoRaw,
    meta: &InterchangeProjectMetadataRaw,
) -> ProjectHash {
    project_hash_str(
        serde_json::to_string(&info).expect("unexpected failure to serialise JSON"),
        serde_json::to_string(&meta).expect("unexpected failure to serialise JSON"),
    )
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use crate::model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw};

    #[test]
    fn str_hash_agrees_with_shell() {
        // cat <(echo -n "foobar") <(echo -n "bazbum") | sha256sum | cut -f 1 -d ' '
        // ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^_ just a fancy way to write echo -n "foobarbazbum"
        //                                              as if concatenated from two separate files
        assert_eq!(
            format!("{:x}", super::project_hash_str("foobar", "bazbum")),
            "e6e2e042d1d461877c7e79cc890af5de00f603739c17486dc1464acfc0f77797"
        );
    }

    #[test]
    fn json_hash_agrees_with_shell() {
        let info = InterchangeProjectInfoRaw {
            name: "json_hash_agrees_with_shell".to_string(),
            publisher: None,
            description: None,
            version: "1.2.3".to_string(),
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        };

        let meta = InterchangeProjectMetadataRaw {
            index: IndexMap::new(),
            created: "0000-00-00T00:00:00.123456789Z".to_string(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        };

        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"name":"json_hash_agrees_with_shell","version":"1.2.3","usage":[]}"#
        );
        assert_eq!(
            serde_json::to_string(&meta).unwrap(),
            r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#
        );

        // cat <(echo -n '{"name":"json_hash_agrees_with_shell","version":"1.2.3","usage":[]}') <(echo -n '{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}') | sha256sum | cut -f 1 -d ' '
        assert_eq!(
            format!("{:x}", super::project_hash_raw(&info, &meta)),
            "b98340d7d7f41cefc3f7dd2b30d65fb48836b12a8d47884975e5c8637edfeea1"
        );
    }
}
