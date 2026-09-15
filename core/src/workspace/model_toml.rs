// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! A workspace read from the `[workspace]` table of a `model.toml`.
//!
//! It is separate from [`crate::workspace`]'s `.workspace.json`
//! workspace, which nothing here uses.
//!
//! What a load does, beyond reading the table, is check the members --
//! workspaces.md "Member requirements" and "Stray per-member state" --
//! and resolve each member's manifest
//! against the table it belongs to. The resolution is not an
//! optimisation: [`crate::manifest::kpar_view::resolved_of_text`]
//! resolves standalone, so a member carrying `version.workspace = true`
//! is otherwise refused *for being a member*.

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

use crate::commands::build::DEFAULT_OUTPUT_NAME;
use crate::commands::lock::DEFAULT_LOCKFILE_NAME;
use crate::env::DEFAULT_ENV_NAME;
use crate::manifest::diagnostics::{Diagnostic, Report};
use crate::manifest::project::ResolvedProject;
use crate::manifest::workspace_table::{MemberPath, WorkspaceTable};
use crate::manifest::{MODEL_TOML_FILE, ModelToml};
use crate::project::ProjectFormat;
use crate::project::local_src::{detect_format, json_manifest};
use crate::project::utils::{FsIoError, ProjectDeserializationError, validate_manifest, wrapfs};

/// Why a load failed, split by the file each half is about.
///
/// A [`Report`] cannot carry both: [`Report::sort_by_position`] orders
/// by byte offset alone and never looks at [`crate::manifest::diagnostics::Position::file`],
/// and [`ProjectDeserializationError::Invalid`] prints one path above a
/// whole report. So the diagnostics that belong to the root manifest and
/// the diagnostics that belong to a member's own manifest are kept
/// apart, each under the name of the file it points into.
///
/// Both halves are filled before the load gives up, so one bad member
/// does not hide the next.
#[derive(Debug)]
pub struct LoadFailure {
    /// Member requirements and stray per-member state, each positioned
    /// at the `members` entry that named the directory. `None` when only members' own files are bad.
    pub root: Option<ProjectDeserializationError>,
    /// Members whose own `model.toml` did not read or did not resolve,
    /// each naming its own file.
    pub members: Vec<ProjectDeserializationError>,
}

impl std::fmt::Display for LoadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for error in self.root.iter().chain(self.members.iter()) {
            if !first {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
            first = false;
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    /// The root's own `model.toml` is not a readable manifest.
    #[error(transparent)]
    Root(ProjectDeserializationError),
    /// It reads, but it defines a project. workspaces.md: "the
    /// `model.toml` that defines the workspace must not also contain a
    /// `[project]` table", so a project there is not a workspace root.
    #[error("`{0}` defines a project, not a workspace")]
    NotAWorkspace(Utf8PathBuf),
    #[error("{0}")]
    Invalid(Box<LoadFailure>),
}

/// One member of a workspace.
#[derive(Debug, Clone)]
pub struct Member {
    /// The `members` entry that named it: both the spelling the author
    /// wrote and the path normalized against the root.
    pub path: MemberPath,
    /// The member's directory, absolute.
    pub dir: Utf8PathBuf,
    /// Its manifest, resolved against the workspace table.
    pub project: ResolvedProject,
}

#[derive(Debug)]
pub struct Workspace {
    root: Utf8PathBuf,
    table: WorkspaceTable,
    members: Vec<Member>,
}

impl Workspace {
    /// Read the `model.toml` at `root` as a workspace, check every
    /// member and resolve each member's manifest against the table.
    ///
    /// `root` is canonicalized, so that [`Workspace::classify`] can
    /// relativize against it and so that messages name one spelling.
    ///
    /// Files are named *relative to the root* in every diagnostic --
    /// `model.toml` for the root, `app/model.toml` for a member -- since
    /// a [`crate::manifest::diagnostics::Position`] displays as
    /// `file:line:column` and three members sharing the bare constant
    /// would be indistinguishable.
    pub fn load(root: impl AsRef<Utf8Path>) -> Result<Self, LoadError> {
        let root = wrapfs::canonicalize(root.as_ref())?;
        let text = wrapfs::read_to_string(root.join(MODEL_TOML_FILE))?;
        let manifest = validate_manifest(MODEL_TOML_FILE, ModelToml::parse(MODEL_TOML_FILE, &text))
            .map_err(LoadError::Root)?;
        let table = match manifest {
            ModelToml::Workspace(table) => *table,
            ModelToml::Project(_) => return Err(LoadError::NotAWorkspace(root)),
        };

        let mut report = Report::new();
        let mut broken = Vec::new();
        let mut members = Vec::with_capacity(table.members.len());
        for path in &table.members {
            if let Some(member) = load_member(&root, path, &table, &mut report, &mut broken)? {
                members.push(member);
            }
        }

        if report.is_empty() && broken.is_empty() {
            return Ok(Self {
                root,
                table,
                members,
            });
        }
        report.sort_by_position();
        Err(LoadError::Invalid(Box::new(LoadFailure {
            root: (!report.is_empty())
                .then(|| ProjectDeserializationError::invalid(MODEL_TOML_FILE, report)),
            members: broken,
        })))
    }

    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    pub fn table(&self) -> &WorkspaceTable {
        &self.table
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }
}

/// Whether `path` is there, and if so whether it is a directory.
///
/// `wrapfs::is_dir` alone cannot tell an absent entry from a file, and
/// the two are different mistakes: a `members` entry naming nothing is a
/// typo or a directory yet to be created, one naming a file is not.
fn entry_kind(path: &Utf8Path) -> Result<Option<bool>, Box<FsIoError>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata.is_dir())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Box::new(FsIoError::IsFile(path.into(), err))),
    }
}

/// workspaces.md "Member requirements", and the read that answers them.
///
/// [`detect_format`] tests file existence, which settles the json, the
/// ambiguous and the empty directory. It cannot settle the fourth case:
/// a `model.toml` holding a `[workspace]` table, one that is not TOML,
/// and one defining neither table all come back [`ProjectFormat::Toml`].
/// Since the member has to be parsed anyway -- its manifest is resolved
/// against the table -- the parse is the check, and
/// [`ModelToml::peek`] is left to the upward walk, where parsing every
/// ancestor would validate files nobody asked about.
fn load_member(
    root: &Utf8Path,
    path: &MemberPath,
    table: &WorkspaceTable,
    report: &mut Report,
    broken: &mut Vec<ProjectDeserializationError>,
) -> Result<Option<Member>, Box<FsIoError>> {
    let raw = &path.raw;
    let dir = root.join(path.path.as_str());

    let mut refuse = |message: String| {
        report.push(Diagnostic::error(message).at_opt(path.at.clone()));
    };

    match entry_kind(&dir)? {
        None => {
            refuse(format!(
                "member `{raw}` does not exist; there is no directory at `{dir}`"
            ));
            return Ok(None);
        }
        Some(false) => {
            refuse(format!("member `{raw}` is not a directory"));
            return Ok(None);
        }
        Some(true) => {}
    }

    // Run whatever the directory being present makes answerable, so that
    // a member with both a stray lockfile and the wrong manifest format
    // reports both.
    stray_state(root, path, &dir, report)?;

    match detect_format(&dir)? {
        ProjectFormat::Json => {
            let json = json_manifest(&dir)?.expect("`Json` means one of the two is there");
            report.push(
                Diagnostic::error(format!(
                    "member `{raw}` uses `{json}`; a workspace member must use the `model.toml` \
                     project definition (`sysand migrate {raw}`)"
                ))
                .at_opt(path.at.clone()),
            );
            Ok(None)
        }
        ProjectFormat::Both => {
            let json = json_manifest(&dir)?.expect("`Both` means one of the two is there");
            // The clause is `LocalSrcError::AmbiguousFormat`'s, written
            // out rather than stringified from it: this message names
            // the member, and folding a typed error into a diagnostic is
            // what `LoadFailure` exists to avoid. A test holds the two
            // together.
            report.push(
                Diagnostic::error(format!(
                    "member `{raw}` has both `model.toml` and `{json}`; it is unclear whether it \
                     is a non-zipped KPAR or a development project"
                ))
                .at_opt(path.at.clone()),
            );
            Ok(None)
        }
        ProjectFormat::Neither => {
            report.push(
                Diagnostic::error(format!("member `{raw}` defines no project"))
                    .at_opt(path.at.clone()),
            );
            Ok(None)
        }
        ProjectFormat::Toml => {
            let file = format!("{}/{MODEL_TOML_FILE}", path.path);
            let text = wrapfs::read_to_string(dir.join(MODEL_TOML_FILE))?;
            let manifest = match validate_manifest(&file, ModelToml::parse(&file, &text)) {
                Ok(manifest) => manifest,
                Err(error) => {
                    broken.push(error);
                    return Ok(None);
                }
            };
            let project = match manifest {
                ModelToml::Project(project) => *project,
                ModelToml::Workspace(_) => {
                    report.push(
                        Diagnostic::error(format!(
                            "member `{raw}` defines a workspace, not a project; a member cannot \
                             be a workspace root"
                        ))
                        .at_opt(path.at.clone()),
                    );
                    return Ok(None);
                }
            };
            match validate_manifest(&file, project.resolve(Some(table))) {
                Ok(project) => Ok(Some(Member {
                    path: path.clone(),
                    dir,
                    project,
                })),
                Err(error) => {
                    broken.push(error);
                    Ok(None)
                }
            }
        }
    }
}

/// workspaces.md "Stray per-member state": "three
/// existence tests, no content is read".
///
/// The message names the *root*, since that is where the state belongs
/// and saying so is the whole content of the rule. Excluded directories
/// are not checked: "Standalone projects under an `exclude`d directory
/// keep their state legitimately; the check applies to members only."
fn stray_state(
    root: &Utf8Path,
    path: &MemberPath,
    dir: &Utf8Path,
    report: &mut Report,
) -> Result<(), Box<FsIoError>> {
    let state = [
        (
            DEFAULT_LOCKFILE_NAME,
            "lockfile",
            wrapfs::is_file(dir.join(DEFAULT_LOCKFILE_NAME))?,
        ),
        (
            DEFAULT_ENV_NAME,
            "environment",
            wrapfs::is_dir(dir.join(DEFAULT_ENV_NAME))?,
        ),
        (
            DEFAULT_OUTPUT_NAME,
            "build output directory",
            wrapfs::is_dir(dir.join(DEFAULT_OUTPUT_NAME))?,
        ),
    ];
    for (name, what, present) in state {
        if present {
            report.push(
                Diagnostic::error(format!(
                    "member `{}` has its own `{name}`; a workspace has one {what} at `{root}`; \
                     remove the member's copy",
                    path.raw
                ))
                .at_opt(path.at.clone()),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "./model_toml_tests.rs"]
mod tests;
