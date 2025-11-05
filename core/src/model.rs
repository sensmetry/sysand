// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{clone::Clone, collections::HashSet, hash::Hash};

#[allow(deprecated)] // will change when `digest` 0.11 is released
use digest::{generic_array::GenericArray, typenum};
use indexmap::IndexMap;
#[cfg(feature = "python")]
use pyo3::{FromPyObject, IntoPyObject};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

// pub struct RawIri(String);
// pub struct ParsedIri(fluent_uri::Iri<String>);
// pub struct NormalisedIri(fluent_uri::Iri<String>);

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Hash, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectUsageG<Iri, VersionReq> {
    pub resource: Iri, // TODO: We should have a fallback for invalid IRIs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_constraint: Option<VersionReq>, // TODO: We should have a fallback for invalid semvars
}
pub type InterchangeProjectUsageRaw = InterchangeProjectUsageG<String, String>;
pub type InterchangeProjectUsage =
    InterchangeProjectUsageG<fluent_uri::Iri<String>, semver::VersionReq>;

impl InterchangeProjectUsageRaw {
    pub fn validate(&self) -> Result<InterchangeProjectUsage, InterchangeProjectValidationError> {
        Ok(InterchangeProjectUsage {
            resource: fluent_uri::Iri::parse(self.resource.clone()).map_err(|e| {
                InterchangeProjectValidationError::IriParse(self.resource.to_string(), e)
            })?,

            version_constraint: self
                .version_constraint
                .as_ref()
                .map(|c| semver::VersionReq::parse(c))
                .transpose()
                .map_err(|e| {
                    InterchangeProjectValidationError::SemverConstraintParse(
                        self.version_constraint.clone().unwrap(),
                        e,
                    )
                })?,
        })
    }
}

impl From<InterchangeProjectUsage> for InterchangeProjectUsageRaw {
    fn from(value: InterchangeProjectUsage) -> InterchangeProjectUsageRaw {
        InterchangeProjectUsageRaw {
            resource: value.resource.to_string(),
            version_constraint: value.version_constraint.map(|x| x.to_string()),
        }
    }
}

impl TryFrom<InterchangeProjectUsageRaw> for InterchangeProjectUsage {
    type Error = InterchangeProjectValidationError;

    fn try_from(value: InterchangeProjectUsageRaw) -> Result<InterchangeProjectUsage, Self::Error> {
        value.validate()
    }
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectInfoG<Iri, Version, VersionReq> {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    pub version: Version, // TODO We should have a fallback for invalid semvars

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

    pub usage: Vec<InterchangeProjectUsageG<Iri, VersionReq>>,
}

pub type InterchangeProjectInfoRaw = InterchangeProjectInfoG<String, String, String>;
pub type InterchangeProjectInfo =
    InterchangeProjectInfoG<fluent_uri::Iri<String>, semver::Version, semver::VersionReq>;

impl From<InterchangeProjectInfo> for InterchangeProjectInfoRaw {
    fn from(value: InterchangeProjectInfo) -> Self {
        InterchangeProjectInfoRaw {
            name: value.name,
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

impl<Iri: PartialEq + Clone, Version, VersionReq: Clone>
    InterchangeProjectInfoG<Iri, Version, VersionReq>
{
    pub fn minimal(name: String, version: Version) -> Self {
        InterchangeProjectInfoG {
            name,
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

    pub fn pop_usage(&mut self, resource: &Iri) -> Vec<InterchangeProjectUsageG<Iri, VersionReq>> {
        // TODO: Once stabilised
        // self.usage.extract_if(.., |InterchangeProjectUsageG { resource: this_resource, .. }| this_resource == resource).collect()

        let (removed, kept): (Vec<_>, Vec<_>) = self
            .usage
            .iter()
            .cloned()
            .partition(
                |InterchangeProjectUsageG {
                     resource: this_resource,
                     ..
                 }| this_resource == resource,
            )
            .to_owned();

        self.usage = kept;

        removed
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
            description: self.description.clone(),
            version: semver::Version::parse(&self.version).map_err(|e| {
                InterchangeProjectValidationError::SemverParse(self.version.as_str().into(), e)
            })?,
            license: self.license.clone(),
            maintainer: self.maintainer.clone(),
            website: self
                .website
                .clone()
                .map(fluent_uri::Iri::parse)
                .transpose()
                .map_err(|e| {
                    InterchangeProjectValidationError::IriParse(self.website.clone().unwrap(), e)
                })?,

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

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectChecksum {
    pub value: String,
    // SHA1, SHA224, SHA256, SHA-384, SHA3-256, SHA3-384, SHA3-512 [SHS]
    // BLAKE2b-256, BLAKE2b-384, BLAKE2b-512, BLAKE3 [BLAKE]
    // MD2, MD4, MD5, MD6 [MD]
    // ADLER32 [ADLER]
    pub algorithm: String,
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectMetadataG<Iri, Path: Eq + Hash, DateTime> {
    pub index: IndexMap<String, Path>,

    pub created: DateTime,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub metamodel: Option<Iri>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub includes_derived: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub includes_implied: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<IndexMap<Path, InterchangeProjectChecksum>>,
}

pub type InterchangeProjectMetadataRaw = InterchangeProjectMetadataG<String, String, String>;
pub type InterchangeProjectMetadata = InterchangeProjectMetadataG<
    fluent_uri::Iri<String>,
    Utf8UnixPathBuf,
    chrono::DateTime<chrono::Utc>,
>;

impl From<InterchangeProjectMetadata> for InterchangeProjectMetadataRaw {
    fn from(value: InterchangeProjectMetadata) -> InterchangeProjectMetadataRaw {
        InterchangeProjectMetadataRaw {
            index: value
                .index
                .iter()
                .map(|(k, v)| (k.to_owned(), v.to_string()))
                .collect(),
            created: value
                .created
                .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            metamodel: value.metamodel.map(|iri| iri.to_string()),
            includes_derived: value.includes_derived,
            includes_implied: value.includes_implied,
            checksum: value.checksum.map(|m| {
                m.iter()
                    .map(|(k, v)| (k.to_string(), v.to_owned()))
                    .collect()
            }),
        }
    }
}

#[derive(Error, Debug)]
pub enum InterchangeProjectValidationError {
    #[error("failed to parse '{0}' as IRI: {1}")]
    IriParse(String, fluent_uri::error::ParseError<String>),
    #[error("failed to parse '{0}' as a Semantic Version: {1}")]
    SemverParse(Box<str>, semver::Error),
    #[error("failed to parse '{0}' as a Semantic Version constraint: {1}")]
    SemverConstraintParse(String, semver::Error),
    #[error("failed to parse '{0}' as RFC3339 datetime: {1}")]
    DatetimeParse(Box<str>, chrono::ParseError),
}

impl InterchangeProjectMetadataRaw {
    pub fn generate_blank() -> Self {
        InterchangeProjectMetadataRaw {
            index: IndexMap::default(),
            created: chrono::Utc::now().to_rfc3339(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }
    }

    pub fn validate(
        &self,
    ) -> Result<InterchangeProjectMetadata, InterchangeProjectValidationError> {
        Ok(InterchangeProjectMetadata {
            index: self
                .index
                .iter()
                .map(|(k, v)| (k.to_owned(), Utf8UnixPath::new(v).to_path_buf()))
                .collect(),
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
                .map(fluent_uri::Iri::parse)
                .transpose()
                .map_err(|e| {
                    InterchangeProjectValidationError::IriParse(self.metamodel.clone().unwrap(), e)
                })?,
            includes_derived: self.includes_derived,
            includes_implied: self.includes_implied,
            checksum: self.checksum.clone().map(|m| {
                m.iter()
                    .map(|(k, v)| (Utf8UnixPath::new(k).to_path_buf(), v.to_owned()))
                    .collect()
            }),
        })
    }

    // TODO: Get rid of overwrite
    /// Adds a checksum to the metadata.
    ///
    /// Overwrites any present value if `overwite`.
    ///
    /// Returns the old checksum value, if present
    pub fn add_checksum<P: AsRef<Utf8UnixPath>, S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        path: P,
        algorithm: S,
        value: T,
        overwrite: bool,
    ) -> Option<InterchangeProjectChecksum> {
        let checksum = if let Some(checksum) = self.checksum.as_mut() {
            checksum
        } else {
            self.checksum = Some(IndexMap::default());
            self.checksum.as_mut().unwrap()
        };

        match checksum.entry(path.as_ref().to_string()) {
            indexmap::map::Entry::Occupied(mut occupied_entry) => Some(if overwrite {
                occupied_entry.insert(InterchangeProjectChecksum {
                    value: value.as_ref().to_string(),
                    algorithm: algorithm.as_ref().to_string(),
                })
            } else {
                occupied_entry.get().clone()
            }),
            indexmap::map::Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(InterchangeProjectChecksum {
                    value: value.as_ref().to_string(),
                    algorithm: algorithm.as_ref().to_string(),
                });

                None
            }
        }
    }

    pub fn remove_checksum<P: AsRef<Utf8UnixPath>>(
        &mut self,
        path: &P,
    ) -> Option<InterchangeProjectChecksum> {
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

impl<Iri, Path: Eq + Hash + Clone, DateTime> InterchangeProjectMetadataG<Iri, Path, DateTime> {
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

#[allow(deprecated)] // will change when `digest` 0.11 is released
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
        serde_json::to_string(&info).expect("Unexpected failure to serialise JSON"),
        serde_json::to_string(&meta).expect("Unexpected failure to serialise JSON"),
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
            "e6e2e042d1d461877c7e79cc890af5de00f603739c17486dc1464acfc0f77797".to_string()
        );
    }

    #[test]
    fn json_hash_agrees_with_shell() {
        let info = InterchangeProjectInfoRaw {
            name: "json_hash_agrees_with_shell".to_string(),
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
            "b98340d7d7f41cefc3f7dd2b30d65fb48836b12a8d47884975e5c8637edfeea1".to_string()
        );
    }
}
