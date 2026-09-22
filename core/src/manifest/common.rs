// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Value types shared by the `[project]` and `[workspace]` sides of a
//! `model.toml`: identifiers, maintainers, inheritance markers, the
//! `stdlib` key and `sources`.

use std::collections::BTreeSet;
use std::fmt::Display;

use fluent_uri::Iri;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::manifest::diagnostics::Diagnostic;
use crate::model::{IndexName, IndexPublisher, KnownMetamodel};
use crate::purl::PKG_SYSAND_PREFIX;
use crate::symbols::Language;

/// The publisher a standard-library usage is resolved under.
///
/// model-toml.md "Standard library": such a usage declares no
/// `publisher`, and "for the dependency graph and the lockfile [it] has
/// the identity `(omg, <name>)`".
pub static OMG_PUBLISHER: std::sync::LazyLock<IndexPublisher> =
    std::sync::LazyLock::new(|| IndexPublisher::parse("omg").expect("`omg` is a valid publisher"));

impl IndexPublisher {
    /// Whether this publisher normalizes to `omg`.
    ///
    /// The `stdlib` key's own check. It is not what makes a usage a
    /// standard-library usage: that is the `kpar-url`, and an index
    /// usage published by `omg` is an ordinary usage.
    pub fn is_stdlib_publisher(&self) -> bool {
        self.normalized() == "omg"
    }
}

/// The normalized identity a project is resolved by (model-toml.md
/// "Normalization"), and the pair the lockfile records as
/// `pkg:sysand/<publisher>/<name>`.
///
/// A `model.toml` usage that declares no publisher is a standard-library
/// usage, whose identity is `(omg, <name>)`, so this pair always has a
/// publisher; a publisher-less json *target* is an error when a toml
/// usage resolves to it.
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

/// The three URLs as concrete values, with no marker left to follow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedUrls {
    pub website: Option<Iri<String>>,
    pub documentation: Option<Iri<String>>,
    pub repository: Option<Iri<String>>,
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
/// separate from [`InheritFrom`] so that a value that is not a marker,
/// `workspace = false` or an invalid group name, can be explained.
/// Serialized untagged, as the bare `true` or `"group"` a file spells.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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

impl From<InheritFrom> for InheritMarker {
    fn from(from: InheritFrom) -> Self {
        match from {
            InheritFrom::DefaultGroup => Self::Flag(true),
            InheritFrom::Group(group) => Self::Group(group),
        }
    }
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
    /// model-toml.md `sources`: "Entries are relative and `/`-separated,
    /// and are taken exactly as written: rather than normalize a path and
    /// then check where it landed, the manifest admits only the one form
    /// Sysand itself writes."
    ///
    /// Nothing is resolved, which is why there is no "escapes the
    /// project root" and no "is the project root": an entry that would
    /// have escaped carries a `..`, and the root is the empty entry.
    #[error(
        "`sources` entry `{0}` is not in the form `sources` takes: entries are relative, \
         `/`-separated,\nand carry no `.` or `..` component and no repeated or leading slash"
    )]
    NotStrict(String),
    /// "an entry with a trailing `/` names a directory and is an error;
    /// directories and globs are postponed to after v1".
    #[error(
        "`sources` entry `{0}` names a directory; every entry names a single file \
         (directories and globs are postponed to after v1)"
    )]
    IsDirectory(String),
    /// "Two equal entries are a duplicate and rejected." Equal, not
    /// equivalent: nothing is resolved, so there is one spelling to
    /// report rather than two.
    #[error("`sources` entry `{0}` is listed more than once")]
    Duplicate(String),
    /// "Every entry names a KerML (`.kerml`) or SysML (`.sysml`) file, the
    /// extension compared case-insensitively ...; any other entry is an
    /// error. Files of other kinds are not listed in `sources`".
    ///
    /// The half of the language rule that needs no metamodel. Which of
    /// the two languages a project may carry is [`admitted_source`],
    /// raised where the metamodel is known --
    /// `a.txt` has no language to name, which is why the two are apart.
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
/// Entries are stored **as written**: "The entry as written is what the
/// `checksum` and `index` maps of a built KPAR are keyed by. The commands
/// that write `sources` (`sysand include`, `sysand migrate`) write this
/// form, converting into it what they are given."
///
/// `sources` "is per-project only: it cannot appear in `[workspace]` and
/// cannot be inherited by a member ..., since the file layout of one
/// project says nothing about another's". Refusing an attempted
/// `sources.workspace = true` with that explanation belongs to whoever
/// reads the table -- [`crate::manifest::project`] for `[project]`,
/// [`crate::manifest::workspace_table`] for a group -- since only a
/// reader knows where the key was written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources(Vec<Utf8UnixPathBuf>);

/// What is wrong with one path that should name a model file relative to
/// the project root: the checks a `sources` entry and a Syside project's
/// `root` share.
///
/// `root` is held to a `sources` entry's syntax, so that there is one
/// path rule in a manifest (design/syside-metamodel.md, "`root`"). The
/// checks are one function so that there is one rule; the *texts* are
/// each key's own ([`SourcesError`], [`RootError`]), because a message
/// about one key cannot speak of "entries".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathProblem {
    /// Ends in `/`.
    Directory,
    /// Contains `\`.
    Backslash,
    /// Contains a NUL.
    Null,
    /// Absolute on either family of operating system.
    Absolute,
    /// Empty, or carrying a `.` or `..` component, or a repeated or
    /// leading slash: not the one form Sysand itself writes.
    NotStrict,
    /// Neither `.kerml` nor `.sysml`.
    NotModelFile,
}

/// The shared checks, in the order [`Sources::new`] has always applied
/// them. `None` for a path that passes all six.
pub fn path_problem(raw: &str) -> Option<PathProblem> {
    if raw.ends_with('/') {
        return Some(PathProblem::Directory);
    }
    if raw.contains('\\') {
        return Some(PathProblem::Backslash);
    }
    if raw.contains('\0') {
        return Some(PathProblem::Null);
    }
    // A Windows drive letter is absolute too, and a test for a leading
    // `/` alone would take `C:/main.sysml` for a relative path.
    if raw.starts_with('/') || typed_path::Utf8WindowsPath::new(raw).is_absolute() {
        return Some(PathProblem::Absolute);
    }
    if raw.is_empty() || raw.split('/').any(|part| matches!(part, "" | "." | "..")) {
        return Some(PathProblem::NotStrict);
    }
    if Language::guess_from_path(Utf8UnixPath::new(raw)).is_none() {
        return Some(PathProblem::NotModelFile);
    }
    None
}

impl SourcesError {
    fn from_problem(problem: PathProblem, raw: &str) -> Self {
        let raw = raw.to_owned();
        match problem {
            PathProblem::Directory => Self::IsDirectory(raw),
            PathProblem::Backslash => Self::ContainsBackslash(raw),
            PathProblem::Null => Self::ContainsNull(raw),
            PathProblem::Absolute => Self::NotRelative(raw),
            PathProblem::NotStrict => Self::NotStrict(raw),
            PathProblem::NotModelFile => Self::NotModelFile(raw),
        }
    }
}

/// Why a Syside project's `root` is not a path a `sources` entry could
/// be: the [`PathProblem`]s, worded for one key.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RootError {
    #[error(
        "`root` `{0}` is absolute; `root` is relative to the project root, like a `sources` entry"
    )]
    Absolute(String),
    #[error(
        "`root` `{0}` contains a backslash `\\`;\n\
         it is `/`-separated, so that a manifest reads the same way on every operating system"
    )]
    Backslash(String),
    #[error("`root` `{0}` contains a NULL character")]
    Null(String),
    #[error(
        "`root` `{0}` is not in the form a `sources` entry takes: relative, `/`-separated,\n\
         with no `.` or `..` component and no repeated or leading slash"
    )]
    NotStrict(String),
    #[error("`root` `{0}` names a directory; `root` names a single model file")]
    Directory(String),
    #[error("`root` `{0}` is not a KerML (`.kerml`) or SysML (`.sysml`) file")]
    NotModelFile(String),
}

impl RootError {
    pub fn new(problem: PathProblem, raw: &str) -> Self {
        let raw = raw.to_owned();
        match problem {
            PathProblem::Directory => Self::Directory(raw),
            PathProblem::Backslash => Self::Backslash(raw),
            PathProblem::Null => Self::Null(raw),
            PathProblem::Absolute => Self::Absolute(raw),
            PathProblem::NotStrict => Self::NotStrict(raw),
            PathProblem::NotModelFile => Self::NotModelFile(raw),
        }
    }
}

/// A Syside project's `root`, checked as a `sources` entry is and kept
/// **as written**. Whether it is one of the `sources` entries is the
/// resolver's question, since only a resolved project knows its family.
pub fn root_path(raw: &str) -> Result<Utf8UnixPathBuf, RootError> {
    match path_problem(raw) {
        Some(problem) => Err(RootError::new(problem, raw)),
        None => Ok(Utf8UnixPathBuf::from(raw)),
    }
}

impl Sources {
    /// Checks every entry against the rules of [`SourcesError`] and
    /// keeps it **as written**. Whether the files exist "is checked only
    /// for build/publish, as with the license file, so reading a
    /// manifest never touches them".
    ///
    /// Nothing is resolved: a `..` is an error rather than a step, so no
    /// entry can escape the project root and none can name it.
    pub fn new(paths: Vec<Utf8UnixPathBuf>) -> Result<Self, SourcesError> {
        Self::check(paths).map_err(|mut problems| problems.swap_remove(0).1)
    }

    /// [`Self::new`], refusing with every entry that breaks a rule, with
    /// its index, in list order; there is at least one. An entry is a
    /// [`SourcesError::Duplicate`] when it repeats an earlier valid one, so
    /// each repeat is reported where it is written; one that is also
    /// invalid is reported as that.
    pub fn check(paths: Vec<Utf8UnixPathBuf>) -> Result<Self, Vec<(usize, SourcesError)>> {
        let mut seen = BTreeSet::new();
        let problems: Vec<_> = paths
            .iter()
            .enumerate()
            .filter_map(|(index, path)| {
                let raw = path.as_str();
                let error = match path_problem(raw) {
                    Some(problem) => SourcesError::from_problem(problem, raw),
                    None if !seen.insert(raw) => SourcesError::Duplicate(raw.to_owned()),
                    None => return None,
                };
                Some((index, error))
            })
            .collect();
        if problems.is_empty() {
            Ok(Self(paths))
        } else {
            Err(problems)
        }
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

/// The half of the language rule the metamodel decides.
///
/// model-toml.md "Metamodel": "The spec kind bounds what `sources` may
/// list [...] An entry the metamodel does not admit is an error naming
/// the entry and the metamodel; the fix is the metamodel that admits it,
/// or a second project for the other language, which is how the standard
/// library is split."
///
/// `Ok(())` for an entry that is not a model file at all: that is
/// [`SourcesError::NotModelFile`]'s, and it needs no metamodel.
pub fn admitted_source(entry: &Utf8UnixPath, metamodel: &KnownMetamodel) -> Result<(), Diagnostic> {
    let Some(language) = Language::guess_from_path(entry) else {
        return Ok(());
    };
    if metamodel.spec.admits(language) {
        return Ok(());
    }
    Err(Diagnostic::error(format!(
        "`sources` entry `{entry}` is a {language} file, which the metamodel `{metamodel}` does \
         not admit;\na KerML project lists `.kerml` files, a SysML project `.sysml` and \
         `.kerml` files"
    )))
}

#[cfg(test)]
#[path = "./common_tests.rs"]
mod tests;
