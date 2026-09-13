// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Value types shared by the `[project]` and `[workspace]` sides of a
//! `model.toml`: identifiers, maintainers, inheritance markers, the
//! `stdlib` key and `sources`.

use std::collections::BTreeSet;
use std::fmt::Display;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use typed_path::Utf8UnixPathBuf;

use crate::manifest::serde_support::PathValue;
use crate::model::{IndexName, IndexPublisher};
use crate::purl::PKG_SYSAND_PREFIX;
use crate::symbols::Language;
use crate::utils::{NormalizeRelativeError, normalize_relative};

impl IndexPublisher {
    /// model-toml.md "Standard library": "A usage is a standard-library
    /// usage iff its publisher normalizes to `omg`".
    pub fn is_stdlib_publisher(&self) -> bool {
        self.normalized() == "omg"
    }
}

/// The normalized identity a project is resolved by (model-toml.md
/// "Normalization"), and the pair the lockfile records as
/// `pkg:sysand/<publisher>/<name>`.
///
/// Unlike a json project's identity, this one always has a publisher.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NormalizedIdentity {
    pub publisher: String,
    pub name: String,
}

impl NormalizedIdentity {
    pub fn lock_identifier(&self) -> String {
        format!("{PKG_SYSAND_PREFIX}{}/{}", self.publisher, self.name)
    }
}

impl Display for NormalizedIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.publisher, self.name)
    }
}

pub fn normalized_identity(publisher: &IndexPublisher, name: &IndexName) -> NormalizedIdentity {
    NormalizedIdentity {
        publisher: publisher.normalized(),
        name: name.normalized(),
    }
}

/// model-toml.md "Standard library": the value of the `stdlib` key, "the
/// last segment of the IRI at which the library's KPAR is published ...
/// validated as a single path segment (`[A-Za-z0-9][A-Za-z0-9-]*`) so it
/// cannot carry `/` or `..` into an IRI". Not inheritable, hence a plain
/// value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StdlibStem(String);

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("`stdlib` value `{0}` is not a single IRI path segment (`[A-Za-z0-9][A-Za-z0-9-]*`)")]
pub struct StdlibStemError(pub String);

impl StdlibStem {
    pub fn parse(raw: impl Into<String>) -> Result<Self, StdlibStemError> {
        let raw = raw.into();
        let mut bytes = raw.bytes();
        let valid = matches!(bytes.next(), Some(b) if b.is_ascii_alphanumeric())
            && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if valid {
            Ok(Self(raw))
        } else {
            Err(StdlibStemError(raw))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for StdlibStem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for StdlibStem {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StdlibStem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(raw).map_err(serde::de::Error::custom)
    }
}

/// model-toml.md only recognizes a single maintainer shape ("array of
/// strings, each of the form `name <email>`; both parts are mandatory"),
/// so this is a plain struct. Each entry stays a string rather than a
/// `{name, email}` table so that the round-trip to a KPAR's
/// `.project.json` `maintainer` array is the identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Maintainer {
    pub name: String,
    pub email: String,
}

/// The only way a maintainer string can fail: it is not `name <email>`
/// with both parts present.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("maintainer `{raw}` is not of the form `name <email>`")]
pub struct MaintainerError {
    pub raw: String,
}

impl Maintainer {
    /// Both parts are mandatory, so a bare name, a bare `<email>`, or
    /// malformed text like `some <some-email> strings` is not a
    /// maintainer at all.
    pub fn parse(raw: &str) -> Result<Self, MaintainerError> {
        let trimmed = raw.trim();
        if let (Some(open), Some(close)) = (trimmed.find('<'), trimmed.rfind('>'))
            && open < close
            && close == trimmed.len() - 1
        {
            let name = trimmed[..open].trim();
            let email = &trimmed[open + 1..close];
            if !name.is_empty() && !email.is_empty() {
                return Ok(Self {
                    name: name.to_owned(),
                    email: email.to_owned(),
                });
            }
        }
        Err(MaintainerError {
            raw: trimmed.to_owned(),
        })
    }
}

impl Display for Maintainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} <{}>", self.name, self.email)
    }
}

impl Serialize for Maintainer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Maintainer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// workspaces.md "Explicit inheritance": which group a `<key>.workspace`
/// marker points at. `workspace = true` selects the implicit default group
/// (keys written directly in `[workspace]`), `workspace = "group-name"` a
/// `[workspace.group.<group-name>]` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InheritFrom {
    DefaultGroup,
    Group(String),
}

impl Display for InheritFrom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DefaultGroup => f.write_str("the default group"),
            Self::Group(group) => write!(f, "group `{group}`"),
        }
    }
}

/// The literal value of a `workspace` marker, before it is checked. Kept
/// separate from [`InheritFrom`] so that the `workspace = false`
/// explanation survives: matching the marker shape with an untagged enum
/// discards the error of every variant it tries.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InheritMarker {
    Flag(bool),
    Group(String),
}

/// workspaces.md "Explicit inheritance": "`<group-name>` is a bare TOML
/// key: letters, digits, `-` and `_`." A marker naming anything else could
/// never find a `[workspace.group.<group-name>]` table.
pub fn is_valid_group_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum InheritMarkerError {
    /// There is no "inherit nothing" marker: a key that is not inherited
    /// simply carries its own value.
    #[error(
        "`workspace = false` is not allowed; a key that is not inherited carries its own value"
    )]
    NotAMarker,
    /// The marker names something that cannot be a group name.
    #[error(
        "`{0}` is not a valid workspace group name (a bare TOML key: letters, digits, \
         `-` and `_`)"
    )]
    InvalidGroupName(String),
}

impl TryFrom<InheritMarker> for InheritFrom {
    type Error = InheritMarkerError;

    fn try_from(marker: InheritMarker) -> Result<Self, Self::Error> {
        match marker {
            InheritMarker::Flag(true) => Ok(Self::DefaultGroup),
            InheritMarker::Flag(false) => Err(InheritMarkerError::NotAMarker),
            InheritMarker::Group(group) if is_valid_group_name(&group) => Ok(Self::Group(group)),
            InheritMarker::Group(group) => Err(InheritMarkerError::InvalidGroupName(group)),
        }
    }
}

impl Serialize for InheritFrom {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        InheritMarkerTable {
            workspace: match self {
                Self::DefaultGroup => InheritMarker::Flag(true),
                Self::Group(group) => InheritMarker::Group(group.clone()),
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for InheritFrom {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(InheritMarkerTable::deserialize(deserializer)?.workspace)
            .map_err(serde::de::Error::custom)
    }
}

/// The `{ workspace = <bool-or-string> }` table an inheritance marker is
/// written as, matched where a plain value would otherwise be. Shared by
/// [`InheritFrom`] (which accepts it) and [`Sources`] (which rejects it).
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InheritMarkerTable {
    pub workspace: InheritMarker,
}

/// Why an entry of `sources` is not one (model-toml.md `sources`).
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SourcesError {
    /// "Entries are relative": an absolute entry has no place in a KPAR.
    #[error("`sources` entry `{0}` is absolute; entries are relative to the project root")]
    NotRelative(String),
    /// A manifest reads the same way on every platform.
    #[error(
        "`sources` entry `{0}` contains a backslash `\\`;\n\
        entries are `/`-separated, so that a manifest reads the same way\n\
        on every operating system"
    )]
    ContainsBackslash(String),
    #[error("`sources` entry `{0}` contains a NULL character")]
    ContainsNull(String),
    /// "An entry that escapes the project root once resolved ... is an
    /// error, so that every listed file has a place in a KPAR."
    #[error("`sources` entry `{0}` escapes the project root, so it has no place in a KPAR")]
    EscapesRoot(String),
    /// The entry resolves to the project root itself, so it names no file.
    #[error("`sources` entry `{0}` is the project root, not a source file")]
    IsRoot(String),
    /// "an entry with a trailing `/` names a directory and is an error;
    /// directories and globs are postponed to after v1".
    #[error(
        "`sources` entry `{0}` names a directory; every entry names a single file \
         (directories and globs are postponed to after v1)"
    )]
    IsDirectory(String),
    /// "Two entries that normalize to the same file are a duplicate and
    /// rejected." Both spellings are reported, since they may differ.
    #[error("{}", duplicate_message(first, second))]
    Duplicate { first: String, second: String },
    /// "Every entry names a KerML (`.kerml`) or SysML (`.sysml`) file, the
    /// extension compared case-insensitively ...; any other entry is an
    /// error. Files of other kinds are not listed in `sources`".
    #[error(
        "`sources` entry `{0}` is not a KerML (`.kerml`) or SysML (`.sysml`) file; \
         `sources` lists model files only"
    )]
    NotModelFile(String),
}

/// model-toml.md `sources`: "array of paths to the project's source files,
/// relative to the project root (the directory containing `model.toml`).
/// Always required for the OMG metamodel: there is no implicit default set
/// of source files, so a project that has none is written as
/// `sources = []`."
///
/// Entries are stored *normalized*: "The normalized form is what the
/// manifest is taken to say: it is what the `checksum` and `index` maps of
/// a built KPAR are keyed by, and what every command that writes `sources`
/// writes."
///
/// `sources` "is per-project only: it cannot appear in `[workspace]` and
/// cannot be inherited by a member ..., since the file layout of one
/// project says nothing about another's". The hand-written `Deserialize`
/// is what turns an attempted `sources.workspace = true` into that
/// explanation rather than a bare type error.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources(Vec<Utf8UnixPathBuf>);

impl Sources {
    /// Normalizes every entry and checks it against the rules of
    /// [`SourcesError`]. Whether the files exist "is checked only for
    /// build/publish, as with the license file, so reading a manifest
    /// never touches them".
    pub fn new(paths: Vec<Utf8UnixPathBuf>) -> Result<Self, SourcesError> {
        let mut normalized: Vec<(Utf8UnixPathBuf, String)> = Vec::with_capacity(paths.len());
        let mut seen = BTreeSet::new();
        for path in &paths {
            let raw = path.as_str();
            if raw.ends_with('/') {
                return Err(SourcesError::IsDirectory(raw.to_owned()));
            }
            let path = normalize_relative(raw).map_err(|e| match e {
                NormalizeRelativeError::Absolute { path: _ } => {
                    SourcesError::NotRelative(raw.to_owned())
                }
                NormalizeRelativeError::ContainsBackslash { path: _ } => {
                    SourcesError::ContainsBackslash(raw.to_owned())
                }
                NormalizeRelativeError::ContainsNull { path: _ } => {
                    SourcesError::ContainsNull(raw.to_owned())
                }
                NormalizeRelativeError::EscapesRoot { path: _ } => {
                    SourcesError::EscapesRoot(raw.to_owned())
                }
                NormalizeRelativeError::IsRoot { path: _ } => SourcesError::IsRoot(raw.to_owned()),
            })?;
            if !seen.insert(path.clone()) {
                let first = normalized
                    .iter()
                    .find(|(seen, _)| *seen == path)
                    .map(|(_, first)| first.clone())
                    .unwrap_or_default();
                return Err(SourcesError::Duplicate {
                    first,
                    second: raw.to_owned(),
                });
            }
            if Language::guess_from_path(&path).is_none() {
                return Err(SourcesError::NotModelFile(raw.to_owned()));
            }
            normalized.push((path, raw.to_owned()));
        }
        Ok(Self(normalized.into_iter().map(|(path, _)| path).collect()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Utf8UnixPathBuf> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_slice(&self) -> &[Utf8UnixPathBuf] {
        &self.0
    }
}

impl Serialize for Sources {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|path| path.as_str()))
    }
}

impl<'de> Deserialize<'de> for Sources {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            List(Vec<PathValue>),
            Inherited(#[expect(dead_code)] InheritMarkerTable),
        }

        match Raw::deserialize(deserializer)? {
            Raw::List(paths) => Self::new(paths.into_iter().map(|p| p.0).collect())
                .map_err(serde::de::Error::custom),
            Raw::Inherited(_) => Err(serde::de::Error::custom(
                "`sources` is per-project and cannot be inherited from the workspace: \
                 every member writes out its own source files in full",
            )),
        }
    }
}

fn duplicate_message(first: &str, second: &str) -> String {
    if first == second {
        format!("`sources` entry `{first}` is listed more than once")
    } else {
        format!("`sources` entries `{first}` and `{second}` name the same file")
    }
}

#[cfg(test)]
#[path = "./common_tests.rs"]
mod tests;
