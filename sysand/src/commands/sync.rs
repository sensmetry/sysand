// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{num::NonZeroU64, sync::Arc};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;
use typed_path::Utf8UnixPathBuf;
use url::ParseError;

use sysand_core::{
    auth::HTTPAuthentication,
    cancel::CancellationToken,
    commands::{
        sync::{MismatchKind, SyncError, SyncOutcome},
        verify::{ManifestUsages, VerifyError, VerifyReport, do_verify},
    },
    env::{WriteEnvironment as _, local_directory::LocalDirectoryEnvironment},
    lock::Lock,
    project::{
        AsSyncProjectTokio, KparMeta, ProjectReadAsync as _,
        gix_git_download::{GixDownloadedError, GixDownloadedProject},
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::LocalSrcProject,
        reqwest_kpar_download::{
            ReqwestIndexKparDownloadedProject, ReqwestRemoteKparDownloadedProject,
        },
        reqwest_src::ReqwestSrcProjectAsync,
        utils::FsIoError,
    },
    utils::ProvidedProjects,
};

/// The `SyncError` instantiation `command_sync` fails with when the sync
/// itself fails.
pub type CliSyncError = SyncError<ParseError, GixDownloadedError, LocalDirectoryEnvironment>;

/// Why `command_sync` failed.
#[derive(Debug, Error)]
pub enum CommandSyncError {
    /// The sync itself, see `SyncError`.
    #[error(transparent)]
    Sync(#[from] CliSyncError),
    /// The sync itself, where the remedy names a flag only the CLI has.
    /// The hint is part of the message so that every printer of this
    /// error carries it.
    #[error("{0}\n       (hint: pass `--include-std` to fetch it from its IRI)")]
    IncludeStd(CliSyncError),
    /// Writing the environment metadata after a completed sync.
    #[error(transparent)]
    WriteMetadata(#[from] Box<FsIoError>),
    /// `--verify` could not read the environment or the manifests.
    #[error(transparent)]
    Verify(#[from] VerifyError),
    /// `--repair` could not remove a mismatched installation.
    #[error("removing `{path}` from the environment: {source}")]
    Remove {
        path: Utf8PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// What `command_sync_verify` found, for the caller's exit code. The
/// findings themselves are in the `SyncOutcome` and were printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyStatus {
    Clean { verified: usize, repaired: usize },
    Problems { count: usize, repaired: usize },
}

/// The CLI's hint for a finding whose remedy is a command core does not
/// name.
fn hint_for(finding: &str, kind: Option<&MismatchKind>, stale: bool) -> String {
    match (kind, stale) {
        (Some(MismatchKind::NotRecorded), _) => format!("{finding}; run `sysand sync`"),
        (_, true) => format!("{finding}; run `sysand lock`"),
        _ => finding.to_owned(),
    }
}

/// Print every finding of `report` as an error line, with hints.
fn print_findings(report: &VerifyReport) {
    for mismatch in &report.mismatched {
        log::error!(
            "{}",
            hint_for(&mismatch.to_string(), Some(&mismatch.kind), false)
        );
    }
    for path in &report.unrecorded {
        log::error!("`{path}` is installed but not recorded in `env.toml`");
    }
    for stale in &report.stale {
        log::error!("{}", hint_for(&stale.to_string(), None, true));
    }
}

fn record(outcome: &mut SyncOutcome, report: &VerifyReport) {
    outcome.verified = report.verified.clone();
    outcome.mismatched = report.mismatched.clone();
    outcome.unrecorded = report.unrecorded.clone();
    outcome.stale = report.stale.clone();
}

/// `sysand sync --verify` / `--repair`.
///
/// Verifies `env` (or, with `None`, the absence of one) against `lock`
/// and `lock` against `manifests`, prints every finding, and with
/// `repair` reinstalls what mismatches and removes what `env.toml` does
/// not record, then verifies again. Stale usages are never
/// repaired: the lockfile is `lock`'s to rewrite. Returns whether the
/// final state is clean; the caller decides the exit code.
#[expect(clippy::too_many_arguments)]
pub fn command_sync_verify<Policy: HTTPAuthentication>(
    lock: &Lock,
    project_root: &Utf8Path,
    env: Option<LocalDirectoryEnvironment>,
    env_dir: &Utf8Path,
    manifests: &[ManifestUsages],
    client: reqwest_middleware::ClientWithMiddleware,
    provided_usages: &ProvidedProjects,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    no_prune: bool,
    repair: bool,
    token: CancellationToken,
    outcome: &mut SyncOutcome,
) -> Result<VerifyStatus, CommandSyncError> {
    let report = do_verify(lock, env.as_ref(), manifests, provided_usages)?;
    record(outcome, &report);
    if !repair {
        print_findings(&report);
        return Ok(if report.is_clean() {
            log::info!(
                "{:>12} `.sysand` matches `sysand-lock.toml`: {} project(s) verified",
                ' ',
                report.verified.len()
            );
            VerifyStatus::Clean {
                verified: report.verified.len(),
                repaired: 0,
            }
        } else {
            VerifyStatus::Problems {
                count: report.problems(),
                repaired: 0,
            }
        });
    }

    // Repair: an environment to repair into, created directly when it
    // is missing (the directory may survive without its `env.toml`).
    let mut env = if let Some(env) = env {
        env
    } else {
        sysand_core::project::utils::wrapfs::create_dir_all(env_dir)?;
        LocalDirectoryEnvironment::create(env_dir)?
    };
    let repairing = "Repairing";
    let header = sysand_core::style::get_style_config().header;
    for mismatch in &report.mismatched {
        match mismatch.kind {
            MismatchKind::Checksum { .. } | MismatchKind::MissingFromDisk => {
                log::info!(
                    "{header}{repairing:>12}{header:#} `{}` {}: {}",
                    mismatch.iri,
                    mismatch.version,
                    mismatch
                );
                env.del_project_version(&mismatch.iri, &mismatch.version)
                    .map_err(|source| CommandSyncError::Remove {
                        path: env_dir.to_path_buf(),
                        source: Box::new(source),
                    })?;
            }
            MismatchKind::NotRecorded => {}
        }
    }
    for path in &report.unrecorded {
        let absolute = env.root_path().join(path.as_str());
        log::info!("{header}{repairing:>12}{header:#} removing unrecorded `{path}`");
        std::fs::remove_dir_all(&absolute).map_err(|source| CommandSyncError::Remove {
            path: absolute.clone(),
            source: Box::new(source),
        })?;
    }
    // A plain sync then installs everything `env.toml` lacks and prunes
    // the rest.
    let mut sync_outcome = SyncOutcome::default();
    command_sync(
        lock,
        project_root,
        &mut env,
        client,
        provided_usages,
        runtime,
        auth_policy,
        no_prune,
        token,
        &mut sync_outcome,
    )?;
    outcome.repaired = sync_outcome.installed;
    outcome.pruned = sync_outcome.pruned;

    let again = do_verify(lock, Some(&env), manifests, provided_usages)?;
    record(outcome, &again);
    print_findings(&again);
    let repaired = outcome.repaired.len();
    Ok(if again.is_clean() {
        log::info!(
            "{:>12} repaired {repaired} project(s); `.sysand` matches `sysand-lock.toml`: {} project(s) verified",
            ' ',
            again.verified.len()
        );
        VerifyStatus::Clean {
            verified: again.verified.len(),
            repaired,
        }
    } else {
        VerifyStatus::Problems {
            count: again.problems(),
            repaired,
        }
    })
}

/// Install the lockfile into `env`. `outcome` is filled in progressively,
/// so on `Err` it holds what was installed and pruned before the failure.
pub fn command_sync<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    lock: &Lock,
    project_root: P,
    env: &mut LocalDirectoryEnvironment,
    client: reqwest_middleware::ClientWithMiddleware,
    provided_usages: &ProvidedProjects,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    no_prune: bool,
    token: CancellationToken,
    outcome: &mut SyncOutcome,
) -> Result<(), CommandSyncError> {
    // `project_root` is the root the lockfile's `src_path`s are relative
    // to: the workspace root for a workspace's lockfile.
    let relative_root = project_root.as_ref();
    sysand_core::commands::sync::do_sync(
        lock,
        env,
        Some(
            |src_path: Utf8UnixPathBuf, checksum: String| -> LocalSrcProject {
                LocalSrcProject::new_access_with_checksum(
                    relative_root.join(src_path.as_str()),
                    Some(src_path),
                    checksum,
                )
            },
        ),
        Some(
            |remote_src: String,
             checksum: String|
             -> Result<AsSyncProjectTokio<ReqwestSrcProjectAsync<Policy>>, ParseError> {
                Ok(ReqwestSrcProjectAsync {
                    client: client.clone(),
                    url: url::Url::parse(&remote_src)?,
                    auth_policy: auth_policy.clone(),
                    expected_checksum: Some(checksum),
                }
                .to_tokio_sync_with(runtime.clone(), token.clone()))
            },
        ),
        Some(
            |kpar_path: Utf8UnixPathBuf,
             kpar_size: NonZeroU64,
             kpar_digest: String|
             -> LocalKParProject {
                LocalKParProject::new_with_expected(
                    relative_root.join(kpar_path.as_str()),
                    KparInnerPath::Guess,
                    Some(kpar_path),
                    KparMeta {
                        size_bytes: kpar_size,
                        sha256_hex: kpar_digest,
                    },
                )
            },
        ),
        // TODO: Fix error handling here
        Some(
            |index_kpar: String,
             index_kpar_size: NonZeroU64,
             index_kpar_digest: String|
             -> Result<
                AsSyncProjectTokio<ReqwestRemoteKparDownloadedProject<Policy>>,
                ParseError,
            > {
                let project = ReqwestRemoteKparDownloadedProject::new_guess_root(
                    url::Url::parse(&index_kpar)?,
                    client.clone(),
                    auth_policy.clone(),
                    Some(KparMeta {
                        size_bytes: index_kpar_size,
                        sha256_hex: index_kpar_digest,
                    }),
                )
                .unwrap();
                Ok(project.to_tokio_sync_with(runtime.clone(), token.clone()))
            },
        ),
        Some(
            |index_kpar: String,
             index_kpar_size: NonZeroU64,
             index_kpar_digest: String|
             -> Result<
                AsSyncProjectTokio<ReqwestIndexKparDownloadedProject<Policy>>,
                ParseError,
            > {
                let project = ReqwestIndexKparDownloadedProject::new(
                    url::Url::parse(&index_kpar)?,
                    client.clone(),
                    auth_policy.clone(),
                    index_kpar_size,
                    index_kpar_digest,
                )
                .unwrap();
                Ok(project.to_tokio_sync_with(runtime.clone(), token.clone()))
            },
        ),
        Some(
            |remote_git: String| -> Result<GixDownloadedProject, GixDownloadedError> {
                GixDownloadedProject::new(remote_git)
                    .map(|project| project.with_cancellation(token.clone()))
            },
        ),
        provided_usages,
        no_prune,
        outcome,
    )
    // A hint that names a command is appended by the CLI: the bindings
    // reach the same error and have no `--include-std` to pass.
    .map_err(|error| match &error {
        SyncError::UnknownStdLib {
            name: _,
            version: _,
        }
        | SyncError::InvalidProvidedVersion {
            iri: _,
            version: _,
            provided_versions: _,
        } => CommandSyncError::IncludeStd(error),
        _ => CommandSyncError::Sync(error),
    })?;

    env.merge_lock(lock);
    env.write()?;

    Ok(())
}
