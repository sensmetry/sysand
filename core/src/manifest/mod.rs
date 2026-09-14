// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The `model.toml` manifest: its value types, the diagnostics reading one
//! produces, and the reader.
//!
//! [`ModelToml::parse`] is the way in. The submodules read the *contents*
//! of a file -- a table, a usage entry, a value -- each given its table
//! and the positions of the file it came from; this module is what
//! decides about the file as a whole: which version of the format it is
//! written for, which of the two tables it defines, and which top-level
//! keys are not known.

pub mod common;
pub mod diagnostics;
pub mod inherit;
pub mod kpar_view;
pub mod project;
pub mod reader;
pub mod spans;
pub mod usage;
pub mod workspace_table;

use crate::manifest::diagnostics::{Diagnostic, KeyLocation, Report, Validated};
use crate::manifest::inherit::own;
use crate::manifest::project::ProjectManifest;
use crate::manifest::reader::TableReader;
use crate::manifest::spans::{Positions, Source};
use crate::manifest::workspace_table::WorkspaceTable;

/// model-toml.md "Manifest version": the highest `manifest-version` this
/// reader supports, and the value generated files carry.
///
/// `i64` is what `toml::Value::Integer` holds, and what
/// `toml_edit::value` takes back when a manifest is written.
pub const SUPPORTED_MANIFEST_VERSION: i64 = 1;

/// The top-level keys of a file that defines a project.
const PROJECT_TOP_KEYS: &[&str] = &["manifest-version", "project", "inline-usages"];

/// The top-level keys of a file that defines a workspace. `inline-usages`
/// is recognized in order to be refused: workspaces.md makes it an error
/// that names the fix, not an unknown key.
const WORKSPACE_TOP_KEYS: &[&str] = &["manifest-version", "workspace", "inline-usages"];

/// Which table a `model.toml` defines, without reading either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelTomlKind {
    Project,
    Workspace,
}

/// A whole `model.toml`, whichever of the two tables it defines.
///
/// model-toml.md: "`model.toml` defines a project, in a `[project]`
/// table. The same file name is used for workspace configuration, in a
/// `[workspace]` table. Exactly one of the two appears in a file: they
/// never appear together, as a workspace root must not contain a project,
/// and a `model.toml` with neither is an error rather than a file that
/// defines nothing -- which is what a mistyped table name would otherwise
/// become."
///
/// Both sides are boxed. Either table is several hundred bytes and a
/// caller holds the enum, not one variant, so a bare enum would carry
/// the larger of the two everywhere; a manifest is built once per file,
/// which is not a rate at which one allocation matters.
#[derive(Clone, Debug)]
pub enum ModelToml {
    Project(Box<ProjectManifest>),
    Workspace(Box<WorkspaceTable>),
}

impl ModelToml {
    /// Read a `model.toml`.
    ///
    /// `file` names the file in messages; nothing here opens one. A
    /// manifest a command built itself, or converted from a json project,
    /// goes through the same validation with
    /// [`Positions::without_spans`].
    ///
    /// Reading accumulates: one report names every problem the file has,
    /// in the order the file reads, warnings among the errors. Three
    /// conditions stop it early, and they have one thing in common --
    /// what the file says about *itself* cannot be acted on, so nothing
    /// after it can be judged:
    ///
    /// 1. the file is not TOML;
    /// 2. `manifest-version` is written but unusable (too new for this
    ///    Sysand, or not an integer). model-toml.md: "It is read before
    ///    anything else, so that a file this Sysand cannot read is
    ///    refused with the right message rather than with a schema
    ///    error";
    /// 3. the file defines both tables, or neither.
    ///
    /// An absent `manifest-version` is *not* one of them: it means no
    /// other schema is in play, so this reader's own applies and the rest
    /// of the file is read as usual. The specification's word for the key
    /// is "read first", which is an order, not a stop -- it names
    /// `publisher`, `name` and `version` in the same sentence.
    pub fn parse(file: &str, text: &str) -> Result<Validated<Self>, Report> {
        let source = Source::new(file, text);
        let (document, positions) = source.read().map_err(|problem| {
            let mut report = Report::new();
            report.push(problem);
            report
        })?;
        let top = TableReader::new(&document, Vec::new(), KeyLocation::TopLevel, &positions);
        let mut report = Report::new();

        if matches!(manifest_version(&source, &top, &mut report), Stop::Yes) {
            return Err(report);
        }

        // Presence, not readability: `project = 3` writes the key
        // `project`, and calling such a file "one that defines neither"
        // would be both false and further from the fix. What is wrong
        // with the value is reported by whoever reads the table.
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
                let manifest = fold(
                    ProjectManifest::validate(&document, &positions),
                    &mut report,
                )
                .map(|manifest| Self::Project(Box::new(manifest)));
                report.extend(top.unknown_keys(PROJECT_TOP_KEYS, None));
                manifest
            }
            (false, true) => {
                let manifest = workspace(&top, &positions, &mut report)
                    .map(|table| Self::Workspace(Box::new(table)));
                report.extend(top.unknown_keys(WORKSPACE_TOP_KEYS, None));
                manifest
            }
        };

        // The two table readers each sort their own report, and the
        // top-level sweep above runs after, so the merged report is
        // sorted once more here.
        report.sort_by_position();
        match manifest {
            Some(manifest) => report.into_result(manifest),
            None => Err(report),
        }
    }

    /// Which table a file defines, without validating either.
    ///
    /// workspaces.md: "The workspace is discovered by walking up from cwd
    /// until a `model.toml` with a `[workspace]` table is encountered".
    /// That walk asks one question of every ancestor, and asking it
    /// through [`ModelToml::parse`] would validate each in full -- so an
    /// unrelated file's missing `manifest-version` would end a walk that
    /// does not care.
    ///
    /// `Ok(None)` is a file with neither table. On the walk that is not
    /// an error, only an answer; whether it is one depends on why the
    /// file was opened.
    ///
    /// A file with *both* tables answers `Workspace`, since that is the
    /// question asked. Such a file is invalid, and the walk learns so
    /// when it reads it.
    pub fn peek(file: &str, text: &str) -> Result<Option<ModelTomlKind>, Diagnostic> {
        let (document, _) = Source::new(file, text).read()?;
        Ok(if document.contains_key("workspace") {
            Some(ModelTomlKind::Workspace)
        } else if document.contains_key("project") {
            Some(ModelTomlKind::Project)
        } else {
            None
        })
    }

    pub fn as_project(&self) -> Option<&ProjectManifest> {
        match self {
            Self::Project(manifest) => Some(manifest),
            Self::Workspace(_) => None,
        }
    }

    pub fn as_workspace(&self) -> Option<&WorkspaceTable> {
        match self {
            Self::Workspace(table) => Some(table),
            Self::Project(_) => None,
        }
    }

    /// The project this file defines, or the error that it defines the
    /// other thing. One function, so that a file opened expecting a
    /// project is refused in the same words wherever it is opened.
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

/// model-toml.md "Manifest version": "The key is mandatory and not
/// inheritable: a member's file must be self-describing before the
/// `[workspace]` table it inherits from is read."
fn manifest_version(source: &Source<'_>, top: &TableReader<'_>, report: &mut Report) -> Stop {
    const KEY: &str = "manifest-version";
    let file = source.file();
    let version = own(top, KEY, "every file carries its own", report, |report| {
        top.integer(KEY, report)
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
        // model-toml.md names two failures, absent and too new, so a
        // version below the supported one is read as this one. It is
        // almost certainly a typo for `1`, but refusing it would be a
        // third rule the specification does not state.
        Some(_) => Stop::No,
        // Written, and unreadable: whoever read it has said why, and
        // what schema the file is in is exactly what is now unknown.
        None if top.has(KEY) => Stop::Yes,
        None => {
            report.push(at_file(
                source,
                format!(
                    "`{file}` has no `{KEY}`; every `model.toml` starts with \
                     `{KEY} = {SUPPORTED_MANIFEST_VERSION}`"
                ),
            ));
            Stop::No
        }
    }
}

fn workspace(
    top: &TableReader<'_>,
    positions: &Positions<'_>,
    report: &mut Report,
) -> Option<WorkspaceTable> {
    // workspaces.md "Config": "A workspace config has no top-level
    // `inline-usages`: that array belongs to a project, and a workspace's
    // common usages are declared under `[workspace]`, so one at the top
    // level of a workspace config is an error." Presence, not a non-empty
    // array: an explicit `inline-usages = []` is the key written.
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
    fold(WorkspaceTable::validate(table.raw(), positions), report)
}

/// Fold one table's result into the file's report.
fn fold<T>(result: Result<Validated<T>, Report>, report: &mut Report) -> Option<T> {
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

/// A diagnostic about the file rather than about anything in it.
///
/// The span index is keyed by paths of at least one segment, so the file
/// as a whole has no span to point at. Left unpositioned such a
/// diagnostic would sort *after* every complaint about a key, which is
/// the wrong end of a report for something that is wrong with the file;
/// [`Source::start`] sorts first.
fn at_file(source: &Source<'_>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(message).at(source.start())
}

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
