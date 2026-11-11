// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Value types shared by the `[project]` and `[workspace]` sides of a
//! `model.toml`: identifiers, maintainers, inheritance markers, the
//! `stdlib` key and `sources`.

use crate::paths::{RelativePathKind, RelativeUnixPathError, parse_relative_unix_path};
use std::collections::BTreeSet;
use std::fmt::Display;

use fluent_uri::Iri;
use serde::Serialize;
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::manifest::diagnostics::Diagnostic;
use crate::model::{IndexName, IndexPublisher, KnownMetamodel};
use crate::purl::{SysandPurlError, parse_sysand_purl, sysand_purl};
use crate::symbols::Language;

/// The publisher a standard-library usage is resolved under.
///
/// Such a usage declares no `publisher`, and for the dependency graph
/// and the lockfile has the identity `(omg, <name>)`.
pub static OMG_PUBLISHER: std::sync::LazyLock<IndexPublisher> =
    std::sync::LazyLock::new(|| IndexPublisher::parse("omg").expect("`omg` is a valid publisher"));

impl IndexPublisher {
    /// Whether this publisher normalizes to `omg`.
    ///
    /// Only the `stdlib` key is checked this way. A usage is a
    /// standard-library usage by its `kpar-url`, and an index usage
    /// published by `omg` is an ordinary usage.
    pub fn is_stdlib_publisher(&self) -> bool {
        self.normalized() == "omg"
    }
}

/// The normalized identity a project is resolved by, and the pair the
/// lockfile records as `pkg:sysand/<publisher>/<name>`.
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
    /// The identity a `pkg:sysand` PURL names: `Ok(None)` for an IRI that
    /// is not one, an error for a malformed one.
    pub fn from_purl(iri: &str) -> Result<Option<Self>, SysandPurlError> {
        Ok(parse_sysand_purl(iri)?.map(|(publisher, name)| Self {
            publisher: publisher.to_owned(),
            name: name.to_owned(),
        }))
    }

    pub fn lock_identifier(&self) -> String {
        sysand_purl(&self.publisher, &self.name)
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

/// The value of the `stdlib` key: the last segment of the IRI at which
/// the library's KPAR is published, validated as a single path segment
/// (`[A-Za-z0-9][A-Za-z0-9-]*`). Not inheritable, hence a plain value.
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

/// A maintainer is a string of the form `name <email>`; both parts are
/// mandatory. Each entry is a string and displays as the text it was
/// parsed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Maintainer {
    pub name: String,
    pub email: String,
    raw: String,
}

/// The only way a maintainer string can fail: it is not `name <email>`
/// with both parts present.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("maintainer `{raw}` is not of the form `name <email>`")]
pub struct MaintainerError {
    pub raw: String,
}

impl Maintainer {
    /// Both parts are mandatory. The name has no surrounding whitespace and is
    /// followed by a single space, the email is a single word with an `@`,
    /// and the string has exactly one `<` and one `>`, which ends it.
    pub fn parse(raw: &str) -> Result<Self, MaintainerError> {
        if let Some((before, rest)) = raw.split_once('<')
            && before.len() >= 2
            && let Some(name) = before.strip_suffix(' ')
            && name.trim() == name
            && let Some(email) = rest.strip_suffix('>')
            && !name.contains('>')
            && email.contains('@')
            && !email.contains(['<', '>'])
            && !email.contains(char::is_whitespace)
        {
            return Ok(Self {
                name: name.to_owned(),
                email: email.to_owned(),
                raw: raw.to_owned(),
            });
        }
        Err(MaintainerError {
            raw: raw.to_owned(),
        })
    }
}

impl Display for Maintainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw)
    }
}

/// The keys `urls` admits.
pub const URL_KEYS: [&str; 3] = ["website", "documentation", "repository"];

/// The `urls` table, one optional `T` per key of [`URL_KEYS`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlsG<T> {
    pub website: Option<T>,
    pub documentation: Option<T>,
    pub repository: Option<T>,
}

/// The three URLs as concrete values, with no marker left to follow.
pub type ResolvedUrls = UrlsG<Iri<String>>;

impl<T> Default for UrlsG<T> {
    fn default() -> Self {
        Self {
            website: None,
            documentation: None,
            repository: None,
        }
    }
}

impl<T> UrlsG<T> {
    /// Each key with its value, in [`URL_KEYS`] order.
    pub fn entries(&self) -> impl Iterator<Item = (&'static str, Option<&T>)> {
        URL_KEYS.into_iter().zip([
            self.website.as_ref(),
            self.documentation.as_ref(),
            self.repository.as_ref(),
        ])
    }
}

/// Which group a `<key>.workspace` marker points at. `workspace = true`
/// selects the implicit default group (keys written directly in
/// `[workspace]`), `workspace = "group-name"` a
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

/// The literal value of a `workspace` marker, before it is checked.
/// Serialized untagged, as the bare `true` or `"group"` a file spells.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum InheritMarker {
    Flag(bool),
    Group(String),
}

/// A `<group-name>` is a bare TOML key: letters, digits, `-` and `_`.
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

/// Why an entry of `sources` is not one.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SourcesError {
    #[error(transparent)]
    Path(PathError),
    /// Two equal entries (compared as written) are a duplicate.
    #[error("`sources` entry `{0}` is listed more than once")]
    Duplicate(String),
}

/// An array of paths to the project's source files, relative to the
/// project root (the directory containing `model.toml`). There is no
/// implicit default set of source files, so a project that has none is
/// written as `sources = []`.
///
/// Entries are stored **as written**: that form is what the `checksum`
/// and `index` maps of a built KPAR are keyed by.
///
/// `sources` is per-project only: it cannot appear in `[workspace]` and
/// cannot be inherited, since it describes one project's file layout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources(Vec<Utf8UnixPathBuf>);

/// What is wrong with one path that should name a model file relative to
/// the project root: the checks a `sources` entry and a Syside project's
/// `root` share.
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
    /// leading slash.
    NotStrict,
    /// Neither `.kerml` nor `.sysml`.
    NotModelFile,
}

/// The first shared check the path fails, or `None` if it passes all of them.
pub fn path_problem(raw: &str) -> Option<PathProblem> {
    if let Err(error) = parse_relative_unix_path(raw, RelativePathKind::SubFile) {
        return Some(match error {
            RelativeUnixPathError::Absolute { path: _ } => PathProblem::Absolute,
            RelativeUnixPathError::ContainsBackslash { path: _ } => PathProblem::Backslash,
            RelativeUnixPathError::ContainsNull { path: _ } => PathProblem::Null,
            RelativeUnixPathError::DirectoryPath { path: _ }
            | RelativeUnixPathError::TrailingSlash { path: _ } => PathProblem::Directory,
            RelativeUnixPathError::ContainsDoubleSlash { path: _ }
            | RelativeUnixPathError::ContainsCurrent { path: _ }
            | RelativeUnixPathError::ContainsParent { path: _ }
            | RelativeUnixPathError::ContainsNonLeadingParent { path: _ }
            | RelativeUnixPathError::EscapesRoot { path: _ }
            | RelativeUnixPathError::IsRoot { path: _ } => PathProblem::NotStrict,
        });
    }
    if Language::guess_from_path(Utf8UnixPath::new(raw)).is_none() {
        return Some(PathProblem::NotModelFile);
    }
    None
}

impl Display for PathProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Directory => {
                "names a directory; it must name a single model file \
                 (directories and globs are not admitted)"
            }
            Self::Backslash => {
                "contains a backslash `\\`;\n\
                 paths are `/`-separated, so that a manifest reads the same way\n\
                 on every operating system"
            }
            Self::Null => "contains a NULL character",
            Self::Absolute => "is absolute; paths are relative to the project root",
            Self::NotStrict => {
                "is not in the form a `sources` entry takes: relative, `/`-separated,\n\
                 with no `.` or `..` component and no repeated or leading slash"
            }
            Self::NotModelFile => "is not a KerML (`.kerml`) or SysML (`.sysml`) file",
        })
    }
}

/// The manifest key a [`PathError`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKey {
    Sources,
    Root,
}

impl Display for PathKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Sources => "`sources` entry",
            Self::Root => "`root`",
        })
    }
}

/// A `sources` entry or a `root` that breaks one of the [`PathProblem`]
/// rules.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{key} `{path}` {problem}")]
pub struct PathError {
    pub key: PathKey,
    pub path: String,
    pub problem: PathProblem,
}

impl PathError {
    pub fn new(key: PathKey, problem: PathProblem, path: &str) -> Self {
        Self {
            key,
            path: path.to_owned(),
            problem,
        }
    }
}

impl SourcesError {
    pub(crate) fn path(problem: PathProblem, raw: &str) -> Self {
        Self::Path(PathError::new(PathKey::Sources, problem, raw))
    }
}

/// A Syside project's `root`, checked as a `sources` entry is and kept
/// **as written**. Whether it is one of the `sources` entries is the
/// resolver's question, since only a resolved project knows its family.
pub fn root_path(raw: &str) -> Result<Utf8UnixPathBuf, PathError> {
    match path_problem(raw) {
        Some(problem) => Err(PathError::new(PathKey::Root, problem, raw)),
        None => Ok(Utf8UnixPathBuf::from(raw)),
    }
}

impl Sources {
    /// Checks every entry against the rules of [`SourcesError`] and
    /// keeps it **as written**. Nothing is resolved (a `..` is an error)
    /// and the files are not touched.
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
                    Some(problem) => SourcesError::path(problem, raw),
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

/// The half of the language rule the metamodel decides.
///
/// The spec kind bounds what `sources` may list. An entry the metamodel
/// does not admit is an error naming the entry and the metamodel; the fix
/// is the metamodel that admits it, or a second project for the other
/// language.
///
/// `Ok(())` for an entry that is not a model file at all: that is
/// [`PathProblem::NotModelFile`]'s, and it needs no metamodel.
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
