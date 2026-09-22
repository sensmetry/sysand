// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `sync --verify`: check the environment against the lockfile by
//! reading what is on disk, not what `env.toml` says about it, and
//! check the lockfile against the manifests.
//!
//! `do_sync` trusts `env.toml`: a lockfile entry whose recorded checksum
//! matches is `kept` without a file under `lib/` being opened. This
//! module re-hashes: the canonical project digest from the manifests on
//! disk against the lockfile's, and every file `.meta.json`'s `checksum`
//! names against its recorded value (which is what a KPAR's archive
//! digest transitively covers, and for the Syside shape is every file
//! but the two manifests). It also lists what `lib/` holds that
//! `env.toml` does not record, and what the manifests use that the
//! lockfile does not lock the same way.
//!
//! Verification changes nothing. Repair is the caller's: remove what
//! mismatches, run `do_sync`, verify again (`sysand sync --repair`).

use camino::Utf8PathBuf;
use semver::{Version, VersionReq};
use thiserror::Error;
use typed_path::Utf8UnixPathBuf;

use crate::{
    commands::sync::{Mismatch, MismatchKind, StaleKind, StaleUsage, SyncedProject},
    env::{
        ProjectChecksumResult, ReadEnvironment as _,
        local_directory::{LocalDirectoryEnvironment, LocalReadError},
    },
    lock::{Lock, Source},
    model::{InterchangeProjectValidationError, KerMlChecksumAlg, project_hash_hex},
    project::{
        ProjectChecksum, ProjectRead as _, hash_reader,
        local_src::{LocalSrcError, LocalSrcProject},
        utils::{FsIoError, Identifier},
    },
    utils::{ProvidedProjects, lowercase_hex},
};

/// One manifest's usages, as the lockfile should reflect them.
#[derive(Debug, Clone)]
pub struct ManifestUsages {
    /// The manifest, for messages (`model.toml` or `.project.json`).
    pub manifest: Utf8PathBuf,
    pub publisher: Option<String>,
    pub name: String,
    pub usages: Vec<ManifestUsage>,
}

#[derive(Debug, Clone)]
pub struct ManifestUsage {
    /// The identifier the lockfile records for this usage.
    pub identifier: Identifier,
    pub constraint: Option<VersionReq>,
}

impl ManifestUsages {
    /// The manifests a command's context covers: every member of the
    /// workspace when there is one, else the current project. Nothing
    /// outside a project.
    pub fn for_context(ctx: &crate::context::ProjectContext) -> Result<Vec<Self>, VerifyError> {
        if let Some(root) = &ctx.model_workspace_root {
            let workspace = crate::workspace::model_toml::Workspace::load_with(
                root,
                crate::workspace::model_toml::MemberChecks::All,
            )?;
            let table = std::sync::Arc::new(workspace.table().clone());
            return workspace
                .members()
                .iter()
                .map(|member| {
                    Self::read(&LocalSrcProject::new_member(
                        member.dir.clone(),
                        None,
                        table.clone(),
                    ))
                })
                .collect();
        }
        ctx.current_project
            .as_ref()
            .map(Self::read)
            .into_iter()
            .collect()
    }

    /// Read a project's manifest.
    pub fn read(project: &LocalSrcProject) -> Result<Self, VerifyError> {
        let Some(info) = project.get_info().map_err(VerifyError::Manifest)? else {
            return Err(VerifyError::Manifest(LocalSrcError::MissingInfoMeta {
                missing: "its manifest",
            }));
        };
        let manifest = match project.format() {
            Ok(crate::project::ProjectFormat::Toml) => {
                project.root_path().join(crate::manifest::MODEL_TOML_FILE)
            }
            _ => project.info_path(),
        };
        let usages = info
            .usage
            .iter()
            .map(|usage| {
                let constraint = usage
                    .validate()
                    .map_err(|source| VerifyError::InvalidUsage {
                        manifest: manifest.clone(),
                        source,
                    })?
                    .version_constraint()
                    .cloned();
                Ok(ManifestUsage {
                    identifier: Identifier::from_interchange_usage_unchecked(usage),
                    constraint,
                })
            })
            .collect::<Result<_, VerifyError>>()?;
        Ok(Self {
            manifest,
            publisher: info.publisher,
            name: info.name,
            usages,
        })
    }
}

/// What a verification found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// Lockfile entries whose installation matches.
    pub verified: Vec<SyncedProject>,
    /// Entries that do not.
    pub mismatched: Vec<Mismatch>,
    /// Directories under `lib/` that `env.toml` does not record,
    /// relative to the environment root.
    pub unrecorded: Vec<Utf8UnixPathBuf>,
    /// Manifest usages the lockfile does not reflect.
    pub stale: Vec<StaleUsage>,
}

impl VerifyReport {
    pub fn is_clean(&self) -> bool {
        self.mismatched.is_empty() && self.unrecorded.is_empty() && self.stale.is_empty()
    }

    pub fn problems(&self) -> usize {
        self.mismatched.len() + self.unrecorded.len() + self.stale.len()
    }

    /// Every finding's text, in report order.
    pub fn findings(&self) -> Vec<String> {
        self.mismatched
            .iter()
            .map(ToString::to_string)
            .chain(
                self.unrecorded
                    .iter()
                    .map(|path| format!("`{path}` is installed but not recorded in `env.toml`")),
            )
            .chain(self.stale.iter().map(ToString::to_string))
            .collect()
    }
}

#[derive(Error, Debug)]
pub enum VerifyError {
    #[error(transparent)]
    EnvRead(#[from] LocalReadError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    Workspace(#[from] crate::workspace::model_toml::LoadError),
    #[error("reading a manifest: {0}")]
    Manifest(LocalSrcError),
    #[error("a usage in `{manifest}` is invalid: {source}")]
    InvalidUsage {
        manifest: Utf8PathBuf,
        source: InterchangeProjectValidationError,
    },
}

/// Check `env` against `lock` and `lock` against `manifests`. `env` is
/// `None` when there is no environment at all (`.sysand` removed):
/// every locked project is then `NotRecorded`. Reads only.
pub fn do_verify(
    lock: &Lock,
    env: Option<&LocalDirectoryEnvironment>,
    manifests: &[ManifestUsages],
    provided: &ProvidedProjects,
) -> Result<VerifyReport, VerifyError> {
    let verifying = "Verifying";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{verifying:>12}{header:#} env");

    let mut report = VerifyReport::default();

    'projects: for project in &lock.projects {
        let Some(iri) = project.identifiers.first() else {
            continue;
        };
        // Provided projects (the standard libraries among them) are not
        // installed, as `do_sync` skips them.
        for id in &project.identifiers {
            if provided.contains_key(&Identifier::from_iri_unchecked_str(id)) {
                continue 'projects;
            }
        }
        if project
            .sources
            .iter()
            .any(|source| matches!(source, Source::Editable { .. }))
        {
            continue;
        }
        // The first source with a checksum is the expectation; a git-only
        // entry has nothing to verify against.
        let Some(expected) = project.sources.iter().find_map(Source::to_checksum) else {
            continue;
        };
        let synced = SyncedProject {
            iri: iri.clone(),
            version: project.version.clone(),
        };
        let mismatch = |kind| Mismatch {
            iri: iri.clone(),
            version: project.version.clone(),
            kind,
        };
        let Some(env) = env else {
            report.mismatched.push(mismatch(MismatchKind::NotRecorded));
            continue;
        };
        match env.has_version_verified(iri, &project.version, &expected)? {
            ProjectChecksumResult::VersionNotFound => {
                report.mismatched.push(mismatch(MismatchKind::NotRecorded));
                continue;
            }
            ProjectChecksumResult::Match => {}
            ProjectChecksumResult::ChecksumNotPresent
            | ProjectChecksumResult::DifferentChecksumKinds
            | ProjectChecksumResult::Mismatch => {
                report
                    .mismatched
                    .push(mismatch(MismatchKind::Checksum { file: None }));
                continue;
            }
        }
        let installed = env.get_project(iri, &project.version)?;
        match verify_installed(&installed, &expected) {
            Ok(None) => report.verified.push(synced),
            Ok(Some(kind)) => report.mismatched.push(mismatch(kind)),
            Err(error) => {
                log::debug!(
                    "`{iri}` {}: reading the installation failed: {error}",
                    project.version
                );
                report
                    .mismatched
                    .push(mismatch(MismatchKind::MissingFromDisk));
            }
        }
    }

    if let Some(env) = env {
        report.unrecorded = env.unrecorded_paths()?;
    }

    for manifest in manifests {
        verify_manifest(lock, manifest, provided, &mut report.stale);
    }

    Ok(report)
}

/// Read one installation back from disk. `None` means it matches.
fn verify_installed(
    installed: &LocalSrcProject,
    expected: &ProjectChecksum,
) -> Result<Option<MismatchKind>, LocalSrcError> {
    if !crate::project::utils::wrapfs::is_dir(installed.root_path())? {
        return Ok(Some(MismatchKind::MissingFromDisk));
    }
    let (Some(info), Some(meta)) = installed.get_project()? else {
        return Ok(Some(MismatchKind::MissingFromDisk));
    };
    // Every file the manifest checksums, hashed from disk. First, so
    // that a changed file is named rather than reported as a changed
    // project digest. A json project installed from a source directory
    // whose author wrote no `checksum` map has nothing here, and only
    // the manifests below are verified for it.
    if meta.checksum.is_none() {
        log::debug!(
            "`{}` has no `checksum` map; only its manifests are verified",
            installed.root_path()
        );
    }
    for (path, checksum) in meta.checksum.iter().flat_map(|map| map.iter()) {
        // Only sha256 entries can be compared; another algorithm's value
        // is folded into the canonical digest below instead.
        if KerMlChecksumAlg::try_from(checksum.algorithm.clone()).ok()
            != Some(KerMlChecksumAlg::Sha256)
        {
            continue;
        }
        let differs = |path: &str| {
            Some(MismatchKind::Checksum {
                file: Some(Utf8UnixPathBuf::from(path)),
            })
        };
        let Ok(mut source) = installed.read_source(path) else {
            return Ok(differs(path));
        };
        let Ok(hash) = hash_reader(&mut source) else {
            return Ok(differs(path));
        };
        if lowercase_hex(hash) != checksum.value.to_lowercase() {
            return Ok(differs(path));
        }
    }
    // The canonical project digest, for a source-installed entry. An
    // archive-installed entry's lockfile digest is the archive's, which
    // the extracted files cannot reproduce: its manifests are covered by
    // the recorded `kpar_cksum` only, so a manifest edited in place under
    // `lib/` is not detected there.
    if let ProjectChecksum::Project(expected_hex) = expected {
        let raw = project_hash_hex(&info, &meta);
        let canonical = match installed.checksum_canonical_hex() {
            Ok(Some(hex)) => hex,
            Ok(None) => return Ok(Some(MismatchKind::MissingFromDisk)),
            Err(crate::project::CanonicalizationError::ProjectRead(error)) => return Err(error),
            Err(crate::project::CanonicalizationError::FileRead(path, _)) => {
                return Ok(Some(MismatchKind::Checksum {
                    file: Some(Utf8UnixPathBuf::from(path.as_ref())),
                }));
            }
        };
        if canonical != *expected_hex && raw != *expected_hex {
            return Ok(Some(MismatchKind::Checksum { file: None }));
        }
    }
    Ok(None)
}

/// The lockfile against one manifest.
fn verify_manifest(
    lock: &Lock,
    manifest: &ManifestUsages,
    provided: &ProvidedProjects,
    stale: &mut Vec<StaleUsage>,
) {
    let entry = lock
        .projects
        .iter()
        .find(|p| p.name == manifest.name && p.publisher == manifest.publisher);
    let Some(entry) = entry else {
        stale.push(StaleUsage {
            manifest: manifest.manifest.clone(),
            usage: manifest.name.clone(),
            kind: StaleKind::ManifestMissing,
        });
        return;
    };
    let stale_usage = |usage: &str, kind| StaleUsage {
        manifest: manifest.manifest.clone(),
        usage: usage.to_owned(),
        kind,
    };
    for usage in &manifest.usages {
        let identifier = usage.identifier.as_str();
        if !entry.usages.iter().any(|u| u.inner() == identifier) {
            stale.push(stale_usage(identifier, StaleKind::Added));
            continue;
        }
        let Some(constraint) = &usage.constraint else {
            continue;
        };
        // The locked dependency: the project the usage names. A provided
        // project (a standard library) is not in the lockfile and has
        // nothing to compare.
        let locked = lock
            .projects
            .iter()
            .find(|p| p.identifiers.iter().any(|id| id == identifier));
        let Some(locked) = locked else {
            if !provided.contains_key(&usage.identifier) {
                stale.push(stale_usage(identifier, StaleKind::Added));
            }
            continue;
        };
        if let Ok(version) = Version::parse(&locked.version)
            && !constraint.matches(&version)
        {
            stale.push(stale_usage(
                identifier,
                StaleKind::Constraint {
                    constraint: constraint.to_string(),
                    locked: locked.version.clone(),
                },
            ));
        }
    }
    for recorded in &entry.usages {
        if !manifest
            .usages
            .iter()
            .any(|u| u.identifier.as_str() == recorded.inner())
        {
            stale.push(stale_usage(recorded.inner(), StaleKind::Removed));
        }
    }
}

#[cfg(test)]
#[path = "./verify_tests.rs"]
mod tests;
