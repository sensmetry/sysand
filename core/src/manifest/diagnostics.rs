// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! What reading a manifest can report: the collected diagnostics a
//! validation produces, and the warnings that do not stop it.
//!
//! Validation collects rather than fails fast, so that one read of a
//! malformed manifest names every problem it has. A validating function
//! returns `Result<Validated<T>, Report>`: the `Report` on the error side
//! lists every problem found, including the warnings, and the
//! `Validated<T>` on the success side carries the value together with the
//! warnings, which the frontend decides how to show.

use std::fmt::Display;
use std::ops::Range;

use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::MetamodelFamily;

/// Whether a diagnostic stops the operation that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
}

impl Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
        })
    }
}

/// Where in which file a diagnostic was produced.
///
/// The line and the column are resolved when the position is built, by
/// the [`crate::manifest::spans::Source`] whose text the span measures;
/// the byte span a TOML document reports is kept alongside them for a
/// renderer that wants to underline the offending key. `serde_json` reports a line and a
/// column and no offset, hence [`Position::at`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position {
    pub file: String,
    /// 1-based.
    pub line: usize,
    /// 1-based, counted in characters.
    pub column: usize,
    /// Byte offsets into the file, when the parser reports them.
    pub span: Option<Range<usize>>,
}

impl Position {
    /// A position reported as a line and a column, as `serde_json` does.
    pub fn at(file: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            file: file.into(),
            line,
            column,
            span: None,
        }
    }
}

impl Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// One problem found while reading a manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub position: Option<Position>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            position: None,
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            position: None,
        }
    }

    /// Attach the position of the offending key or value.
    #[must_use]
    pub fn at(mut self, position: Position) -> Self {
        self.position = Some(position);
        self
    }

    /// Attach a position if there is one to attach.
    ///
    /// A diagnostic about a key that is *missing* has no key to point at,
    /// and a manifest that was not read from text has no positions at
    /// all; both leave the diagnostic where it is rather than inventing
    /// somewhere for it to be.
    #[must_use]
    pub fn at_opt(self, position: Option<Position>) -> Self {
        match position {
            Some(position) => self.at(position),
            None => self,
        }
    }
}

impl Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.position {
            Some(position) => write!(f, "{position}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Every diagnostic one read produced, in the order they were found.
///
/// A report holds errors and warnings together so that they can be shown
/// in the order of the file; [`Report::into_result`] is what decides
/// whether the read failed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report(Vec<Diagnostic>);

impl Report {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.0.push(diagnostic);
    }

    pub fn extend(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        self.0.extend(diagnostics);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.0
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Warning)
    }

    /// The diagnostics, for a caller that folds one report into another.
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.0
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.0
    }

    /// Put the diagnostics in the order the file reads.
    ///
    /// Validation finds problems in the order its checks run - field
    /// order for the keys a table knows, then the ones it does not - which
    /// is not the order the reader works through the file in. Sorting once at the
    /// end of a read is what makes a report with several problems
    /// something that can be fixed from top to bottom.
    ///
    /// The sort is stable, so two diagnostics about the same key keep the
    /// order the checks found them in, and a diagnostic with no position
    /// sorts after every diagnostic that has one.
    pub fn sort_by_position(&mut self) {
        self.0.sort_by_key(|diagnostic| {
            diagnostic
                .position
                .as_ref()
                .map_or((1, usize::MAX, usize::MAX), |position| {
                    match &position.span {
                        Some(span) => (0, span.start, 0),
                        None => (0, usize::MAX, position.line),
                    }
                })
        });
    }

    /// The end of every validating function: `value` is only produced if
    /// nothing in the report is an error, and the warnings travel with it.
    pub fn into_result<T>(self, value: T) -> Result<Validated<T>, Self> {
        if self.has_errors() {
            return Err(self);
        }
        Ok(Validated {
            value,
            warnings: self.0,
        })
    }
}

impl FromIterator<Diagnostic> for Report {
    fn from_iter<I: IntoIterator<Item = Diagnostic>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, diagnostic) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{diagnostic}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Report {}

/// A value that was read successfully, and the warnings its read produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validated<T> {
    pub value: T,
    pub warnings: Vec<Diagnostic>,
}

impl<T> Validated<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            warnings: Vec::new(),
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Validated<U> {
        Validated {
            value: f(self.value),
            warnings: self.warnings,
        }
    }

    pub fn into_parts(self) -> (T, Vec<Diagnostic>) {
        (self.value, self.warnings)
    }
}

/// workspaces.md "Unknown keys": unknown keys are allowed for future
/// compatibility, "but generate a warning upon read. This holds for
/// `model.toml` as a whole, `[project]` and `[workspace]` alike, and for
/// the `urls` tables." Serde's default is to drop unrecognized keys
/// silently, which cannot produce that warning, and `deny_unknown_fields`
/// would turn it into an error -- so a table that has to warn keeps a
/// flattened `UnknownKeys` that remembers the names it did not recognize,
/// in the order it read them.
///
/// Only the keys are kept: the values have no home in the type map, and
/// nothing writes them back out (serializing an `UnknownKeys` emits an
/// empty table, so a flattened one contributes no keys).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnknownKeys(Vec<String>);

impl FromIterator<String> for UnknownKeys {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl UnknownKeys {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// One warning per unrecognized key, attributed to where it was
    /// found.
    pub fn warnings(&self, location: &KeyLocation) -> Vec<ReadWarning> {
        self.warnings_with_metamodel(location, None)
    }

    /// The same, with the metamodel the "this may be because" suffix
    /// names: an unrecognized metamodel may be why the key is not known.
    pub fn warnings_with_metamodel(
        &self,
        location: &KeyLocation,
        metamodel: Option<&str>,
    ) -> Vec<ReadWarning> {
        self.iter()
            .map(|key| ReadWarning::UnknownKey {
                location: location.clone(),
                key: key.to_owned(),
                unrecognized_metamodel: metamodel.map(str::to_owned),
            })
            .collect()
    }
}

impl Serialize for UnknownKeys {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(std::iter::empty::<(&str, &str)>())
    }
}

impl<'de> Deserialize<'de> for UnknownKeys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeysVisitor;

        impl<'de> Visitor<'de> for KeysVisitor {
            type Value = UnknownKeys;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "a table")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut keys = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    map.next_value::<IgnoredAny>()?;
                    keys.push(key);
                }
                Ok(UnknownKeys(keys))
            }
        }

        deserializer.deserialize_map(KeysVisitor)
    }
}

/// Where a key was found, or was expected: a table of a `model.toml`, the
/// top level of one, or a json manifest, which has no tables to name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyLocation {
    /// A `[table]` of a `model.toml`, named without its brackets
    /// (`project`, `workspace.group.legacy`).
    Table(String),
    /// The top level of a `model.toml`.
    TopLevel,
    /// An array of tables of a `model.toml`, named without its brackets
    /// (`inline-usages`, `workspace.group.legacy.inline-usages`).
    ArrayTable(String),
    /// A json manifest, named by its file name (`.project.json`).
    File(String),
}

impl Display for KeyLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Table(table) => write!(f, "`[{table}]`"),
            Self::ArrayTable(table) => write!(f, "`[[{table}]]`"),
            Self::TopLevel => f.write_str("the top level"),
            Self::File(file) => write!(f, "`{file}`"),
        }
    }
}

/// A key the manifest must carry is not there, and an
/// unrecognized metamodel may be the reason.
///
/// Both manifest formats report a missing key through this function, so
/// that the two cannot drift apart.
pub fn missing_key(key: &str, location: &KeyLocation, metamodel: Option<&str>) -> Diagnostic {
    let mut message = format!("missing key `{key}` in {location}");
    if let Some(url) = metamodel {
        message.push_str(&unsupported_metamodel_suffix(url));
    }
    Diagnostic::error(message)
}

/// What reading a manifest can report without failing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadWarning {
    /// workspaces.md "Unknown keys". `unrecognized_metamodel` is set when
    /// the project declares a metamodel outside both families
    /// (model-toml.md "Metamodel", rule (a)): the key may then belong to
    /// an unsupported schema, and the warning says so.
    UnknownKey {
        location: KeyLocation,
        key: String,
        unrecognized_metamodel: Option<String>,
    },
    /// model-toml.md "Metamodel": "a key that belongs to another
    /// metamodel's schema (`root` in an OMG-metamodel project) is a
    /// warning naming the metamodel that supports it".
    ForeignSchemaKey {
        location: KeyLocation,
        key: String,
        supported_by: MetamodelFamily,
    },
    /// model-toml.md "Metamodel": "Any other value is a warning, not an
    /// error."
    UnknownMetamodel {
        location: KeyLocation,
        value: String,
    },
    /// workspaces.md "Source overrides": "A `sysand.toml` anywhere else
    /// under the workspace is ignored as a whole ... with a warning on any
    /// operation touching the overrides."
    IgnoredSourceOverrides { path: String },
}

impl ReadWarning {
    /// The warning as a diagnostic pointing at the key that produced it.
    pub fn at(self, position: Position) -> Diagnostic {
        Diagnostic::from(self).at(position)
    }
}

impl From<ReadWarning> for Diagnostic {
    fn from(warning: ReadWarning) -> Self {
        Self::warning(warning.to_string())
    }
}

impl Display for ReadWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKey {
                location,
                key,
                unrecognized_metamodel,
            } => {
                if *location == KeyLocation::TopLevel {
                    write!(f, "unknown top-level key `{key}`")?;
                } else {
                    write!(f, "unknown key `{key}` in {location}")?;
                }
                if let Some(url) = unrecognized_metamodel {
                    write!(f, "{}", unsupported_metamodel_suffix(url))?;
                }
                Ok(())
            }
            Self::ForeignSchemaKey {
                location,
                key,
                supported_by,
            } => write!(
                f,
                "`{key}` in {location} is only supported by the {supported_by} metamodel \
                 and is ignored here"
            ),
            Self::UnknownMetamodel { location, value } => write!(
                f,
                "{location} declares metamodel `{value}`, which this version of Sysand does \
                 not recognize"
            ),
            Self::IgnoredSourceOverrides { path } => write!(
                f,
                "`{path}` is not at the workspace root, so it is ignored as a whole \
                 (source overrides and `[[index]]` configuration alike)"
            ),
        }
    }
}

/// model-toml.md "Metamodel", rule (a): a manifest that declares a
/// metamodel outside both recognized families is validated against the OMG
/// schema, and every schema complaint says that the metamodel may be the
/// reason. One function, so that every message naming such a metamodel
/// reads the same.
pub fn unsupported_metamodel_suffix(metamodel: &str) -> String {
    format!(
        "; this may be because the project uses a metamodel this version of Sysand does not \
         support: `{metamodel}`"
    )
}

#[cfg(test)]
#[path = "./diagnostics_tests.rs"]
mod tests;
