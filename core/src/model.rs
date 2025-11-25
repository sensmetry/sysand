// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{clone::Clone, collections::HashSet, fmt::Display, hash::Hash};

#[allow(deprecated)] // will change when `digest` 0.11 is released
use digest::{generic_array::GenericArray, typenum};
use indexmap::IndexMap;
#[cfg(feature = "python")]
use pyo3::{FromPyObject, IntoPyObject, pyclass};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

// pub struct RawIri(String);
// pub struct ParsedIri(fluent_uri::Iri<String>);
// pub struct NormalisedIri(fluent_uri::Iri<String>);

pub const KNOWN_METAMODELS: [&str; 6] = [
    "https://www.omg.org/spec/SysML/20250201",
    "https://www.omg.org/spec/SysML/20240201",
    "https://www.omg.org/spec/SysML/20230201",
    "https://www.omg.org/spec/KerML/20250201",
    "https://www.omg.org/spec/KerML/20240201",
    "https://www.omg.org/spec/KerML/20230201",
];

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Hash, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectUsageG<Iri, VersionReq> {
    pub resource: Iri, // TODO: We should have a fallback for invalid IRIs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_constraint: Option<VersionReq>, // TODO: We should have a fallback for invalid semvers
}
pub type InterchangeProjectUsageRaw = InterchangeProjectUsageG<String, String>;
pub type InterchangeProjectUsage =
    InterchangeProjectUsageG<fluent_uri::Iri<String>, semver::VersionReq>;

impl InterchangeProjectUsageRaw {
    pub fn validate(&self) -> Result<InterchangeProjectUsage, InterchangeProjectValidationError> {
        Ok(InterchangeProjectUsage {
            resource: fluent_uri::Iri::parse(self.resource.clone())
                .map_err(|(e, val)| InterchangeProjectValidationError::IriParse(val, e))?,

            version_constraint: self
                .version_constraint
                .as_ref()
                .map(|c| semver::VersionReq::parse(c))
                .transpose()
                .map_err(|e| {
                    InterchangeProjectValidationError::SemVerConstraintParse(
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
        // TODO: MSRV >=1.87
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
#[cfg_attr(feature = "python", pyclass(eq, eq_int))]
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
    pub fn expected_hex_len(&self) -> u8 {
        use KerMlChecksumAlg::*;
        match self {
            None => 0,
            Sha1 => 40,
            Sha224 => 56,
            Sha256 | Sha3_256 | Blake2b256 | Blake3 => 64,
            Sha384 | Sha3_384 | Blake2b384 => 96,
            Sha3_512 | Blake2b512 => 128,
            Md2 | Md4 | Md5 => 32,
            // MD6 is variable length. TODO(spec): the
            // digest length must be somehow specified
            // Maybe specify as default MD6-256
            Md6 => todo!("MD6 is variable length"),
            Adler32 => 8,
        }
    }
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProjectChecksum {
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
                    .map(|(k, v)| {
                        (
                            k.to_string(),
                            InterchangeProjectChecksumRaw {
                                value: v.value.clone(),
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
        let checksum = if let Some(checksum) = &self.checksum {
            let mut res = IndexMap::with_capacity(checksum.len());
            for (k, v) in checksum {
                let k = Utf8UnixPath::new(k).to_path_buf();
                let algorithm: KerMlChecksumAlg =
                    v.algorithm.as_str().try_into().map_err(|_| {
                        InterchangeProjectValidationError::ChecksumAlg(v.algorithm.as_str().into())
                    })?;
                let value = {
                    let expected_len = algorithm.expected_hex_len();
                    if v.value.len() != expected_len as usize {
                        return Err(InterchangeProjectValidationError::ChecksumLen {
                            algorithm,
                            expected: expected_len,
                            got: v.value.len(),
                        });
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
        let checksum = if let Some(checksum) = self.checksum.as_mut() {
            checksum
        } else {
            self.checksum = Some(IndexMap::default());
            self.checksum.as_mut().unwrap()
        };

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
