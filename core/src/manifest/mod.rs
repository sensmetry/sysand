// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The `model.toml` manifest: its value types, the diagnostics reading one
//! produces, and the reader.
//!
//! [`ModelToml::parse`] is the way in. The submodules read the *contents*
//! of a file (a table, a usage entry, a value), each given a reader of
//! its table, which knows where in the file it is; this module is what
//! decides about the file as a whole: which version of the format it is
//! written for, which of the two tables it defines, and which top-level
//! keys are not known.

pub mod common;
pub mod diagnostics;
pub mod edit;
pub mod inherit;
pub mod kpar_view;
pub mod project;
pub mod reader;
pub mod spans;
pub mod usage;
pub mod workspace_table;

use toml::Spanned;
use toml::de::DeValue;

use crate::manifest::diagnostics::{Diagnostic, Diagnostics, KeyLocation, Validated};
use crate::manifest::inherit::own;
use crate::manifest::project::ProjectManifest;
use crate::manifest::reader::TableReader;
use crate::manifest::spans::Source;
use crate::manifest::workspace_table::WorkspaceTable;

/// The name of the manifest this module reads.
pub const MODEL_TOML_FILE: &str = "model.toml";

/// A `manifest-version`: `<major>.<minor>`, ordered by major, then minor.
///
/// Additions bump the minor, an incompatible change the major.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ManifestVersion {
    major: u32,
    minor: u32,
}

impl ManifestVersion {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Two plain decimal numbers around a dot: no sign, no leading zeros,
    /// no further components.
    fn parse(text: &str) -> Option<Self> {
        fn number(text: &str) -> Option<u32> {
            let plain = !text.is_empty()
                && text.bytes().all(|b| b.is_ascii_digit())
                && (text == "0" || !text.starts_with('0'));
            plain.then(|| text.parse().ok()).flatten()
        }
        let (major, minor) = text.split_once('.')?;
        Some(Self::new(number(major)?, number(minor)?))
    }
}

impl std::fmt::Display for ManifestVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// The highest `manifest-version` this reader supports, and the value
/// generated files carry.
pub const SUPPORTED_MANIFEST_VERSION: ManifestVersion = ManifestVersion::new(1, 0);

/// The top-level keys of a file that defines a project.
const PROJECT_TOP_KEYS: &[&str] = &["manifest-version", "project", "inline-usages"];

/// The top-level keys of a file that defines a workspace. `inline-usages`
/// is recognized in order to be refused with an error that names the
/// fix, not treated as an unknown key.
const WORKSPACE_TOP_KEYS: &[&str] = &["manifest-version", "workspace", "inline-usages"];

/// Which table a `model.toml` defines, without reading either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelTomlKind {
    Project,
    Workspace,
}

/// A whole `model.toml`, whichever of the two tables it defines.
///
/// A `model.toml` defines a project in a `[project]` table or a
/// workspace in a `[workspace]` table. Exactly one of the two appears in
/// a file: they never appear together, as a workspace root must not
/// contain a project, and a `model.toml` with neither is an error.
///
/// Both sides are boxed to keep the enum small.
#[derive(Clone, Debug)]
pub enum ModelToml {
    Project(Box<ProjectManifest>),
    Workspace(Box<WorkspaceTable>),
}

impl ModelToml {
    /// Read a `model.toml`.
    ///
    /// `file` names the file in messages; nothing here opens one.
    ///
    /// Reading accumulates: one report names every problem the file has,
    /// in the order the file reads, warnings among the errors. Three
    /// conditions stop it early: what the file says about *itself* cannot be
    /// acted on, so nothing after it can be judged:
    ///
    /// 1. the file is not TOML;
    /// 2. `manifest-version` is written but unusable (too new for this
    ///    Sysand, or not a `<major>.<minor>` string). It is read before anything
    ///    else;
    /// 3. the file defines both tables, or neither.
    ///
    /// An absent `manifest-version` is *not* one of them: no other schema is
    /// in play, so this reader's own applies and the rest of the file is
    /// read as usual.
    pub fn parse(file: &str, text: &str) -> Result<Validated<Self>, Diagnostics> {
        let source = Source::new(file, text);
        let document = source.read().map_err(|problem| {
            let mut report = Diagnostics::new();
            report.push(problem);
            report
        })?;
        let top = TableReader::new(document.get_ref(), None, KeyLocation::TopLevel, &source);
        let mut report = Diagnostics::new();

        if matches!(manifest_version(&source, &top, &mut report), Stop::Yes) {
            return Err(report);
        }

        // Presence, not readability: `project = 3` writes the key
        // `project`. What is wrong with the value is reported by whoever
        // reads the table.
        let manifest = match (top.has("project"), top.has("workspace")) {
            (true, true) => {
                report.push(at_file(
                    &source,
                    format!(
                        "`{file}` defines both `[project]` and `[workspace]`;\n\
                             a workspace root must not contain a project"
                    ),
                ));
                return Err(report);
            }
            (false, false) => {
                report.push(at_file(
                    &source,
                    format!(
                        "`{file}` defines neither `[project]` nor `[workspace]`;\n\
                         a `model.toml` defines one or the other"
                    ),
                ));
                return Err(report);
            }
            (true, false) => {
                let manifest = fold(ProjectManifest::validate(&top), &mut report)
                    .map(|manifest| Self::Project(Box::new(manifest)));
                report.extend(top.unknown_keys(PROJECT_TOP_KEYS, None));
                manifest
            }
            (false, true) => {
                let manifest =
                    workspace(&top, &mut report).map(|table| Self::Workspace(Box::new(table)));
                report.extend(top.unknown_keys(WORKSPACE_TOP_KEYS, None));
                manifest
            }
        };

        // The table readers sort their own reports; sort the merge again.
        report.sort_by_position();
        match manifest {
            Some(manifest) => report.into_result(manifest),
            None => Err(report),
        }
    }

    /// Which table a file defines, without validating either.
    ///
    /// Meant for walks over ancestor files: an unrelated file's missing
    /// `manifest-version` must not end the walk.
    ///
    /// `Ok(None)` is a file with neither table.
    ///
    /// A file with *both* tables answers `Workspace`; reading it reports the
    /// error.
    pub fn peek(file: &str, text: &str) -> Result<Option<ModelTomlKind>, Diagnostic> {
        let document = Source::new(file, text).read()?;
        let document = document.get_ref();
        Ok(if document.contains_key("workspace") {
            Some(ModelTomlKind::Workspace)
        } else if document.contains_key("project") {
            Some(ModelTomlKind::Project)
        } else {
            None
        })
    }

    /// The project this file defines, or the error that it defines the
    /// other thing.
    pub fn into_project(self, file: &str) -> Result<ProjectManifest, Diagnostic> {
        match self {
            Self::Project(manifest) => Ok(*manifest),
            Self::Workspace(_) => Err(Diagnostic::error(format!(
                "`{file}` defines a workspace, not a project"
            ))),
        }
    }

    pub fn into_workspace(self, file: &str) -> Result<WorkspaceTable, Diagnostic> {
        match self {
            Self::Workspace(table) => Ok(*table),
            Self::Project(_) => Err(Diagnostic::error(format!(
                "`{file}` defines a project, not a workspace"
            ))),
        }
    }
}

/// Whether the read can go on.
enum Stop {
    Yes,
    No,
}

/// The key is mandatory and not inheritable: a member's file must be
/// self-describing before the `[workspace]` table it inherits from is
/// read.
fn manifest_version(source: &Source<'_>, top: &TableReader<'_>, report: &mut Diagnostics) -> Stop {
    const KEY: &str = "manifest-version";
    let file = source.file();
    let version = own(top, KEY, "every file carries its own", report, |report| {
        read_version(source, top, KEY, report)
    });
    match version {
        Some(found) if found > SUPPORTED_MANIFEST_VERSION => {
            report.push(at_file(
                source,
                format!(
                    "`{file}` is written for manifest-version {found}; this Sysand supports \
                     up to {SUPPORTED_MANIFEST_VERSION}; update Sysand"
                ),
            ));
            Stop::Yes
        }
        // Only absent and too new are failures, so a version below the
        // supported one is read as this one.
        Some(_) => Stop::No,
        // Written but unusable (already reported): the schema of the file
        // is unknown.
        None if top.has(KEY) => Stop::Yes,
        // Absent (already reported): no other schema is in play, so the
        // rest of the file is still judged.
        None => Stop::No,
    }
}

/// The version the file states, or the error that it states none.
///
/// One message covers three failures: absent, not a string, and a string
/// that is not a `<major>.<minor>` version. The value is quoted as the
/// file spells it.
///
/// The error is reported here, not by the caller, so that an inheritance
/// marker (which skips this function) does not get a second error.
fn read_version(
    source: &Source<'_>,
    top: &TableReader<'_>,
    key: &str,
    report: &mut Diagnostics,
) -> Option<ManifestVersion> {
    let found = top.get(key);
    if let Some(DeValue::String(written)) = found.map(Spanned::get_ref)
        && let Some(version) = ManifestVersion::parse(written)
    {
        return Some(version);
    }
    let spelled = match found {
        Some(value) => format!(" (`{}`)", source.spelled(value.span())),
        None => String::new(),
    };
    let file = source.file();
    let message = format!(
        "`{file}` has no usable `{key}`{spelled}; every `model.toml` starts with \
         `{key} = \"{SUPPORTED_MANIFEST_VERSION}\"`"
    );
    // A written key has a place to point at; an absent one is a problem
    // with the file as a whole, and points at its start.
    report.push(match found {
        Some(_) => Diagnostic::error(message).at_opt(top.at(key)),
        None => at_file(source, message),
    });
    None
}

fn workspace(top: &TableReader<'_>, report: &mut Diagnostics) -> Option<WorkspaceTable> {
    // Presence, not a non-empty array: an explicit `inline-usages = []`
    // is an error too.
    if top.has("inline-usages") {
        report.push(
            Diagnostic::error(
                "a workspace config has no top-level `inline-usages`; declare common usages \
                 under `[workspace]`",
            )
            .at_opt(top.at("inline-usages")),
        );
    }
    let table = top.table("workspace", report)?;
    fold(WorkspaceTable::validate(&table), report)
}

/// Fold one table's result into the file's report.
fn fold<T>(result: Result<Validated<T>, Diagnostics>, report: &mut Diagnostics) -> Option<T> {
    match result {
        Ok(validated) => {
            let (value, warnings) = validated.into_parts();
            report.extend(warnings);
            Some(value)
        }
        Err(problems) => {
            report.extend(problems.into_diagnostics());
            None
        }
    }
}

/// A diagnostic about the file itself.
///
/// The file as a whole has no span to point at. Such a diagnostic is
/// positioned at [`Source::start`], which sorts first.
fn at_file(source: &Source<'_>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(message).at(source.start())
}

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
