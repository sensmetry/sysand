// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use pubgrub::Reporter as _;

use sysand_core::{
    commands::lock::{
        DEFAULT_LOCKFILE_NAME, LockError, LockOutcome, LockProjectError, do_lock_local_editable,
    },
    config::Config,
    project::{ProjectRead, memory::InMemoryProject, utils::wrapfs},
    resolve::{
        ResolveRead,
        memory::{AcceptAll, MemoryResolver},
        priority::PriorityResolver,
        standard::{StandardResolver, standard_resolver},
    },
    solve::pubgrub::{DependencyIdentifier, InternalSolverError},
    stdlib::known_std_libs,
};

use crate::{DEFAULT_INDEX_URL, cli::ResolutionOptions};

/// Generate a lockfile for project at `path`.
/// `path` must be relative to workspace root.
// TODO: this will not work properly if run in subdir of workspace,
// as `path` will then refer to a deeper subdir
pub fn command_lock<P: AsRef<Path>>(
    path: P,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<sysand_core::lock::Lock> {
    assert!(path.as_ref().is_relative(), "{}", path.as_ref().display());

    let provided_iris = if !resolution_opts.include_std {
        known_std_libs()
    } else {
        HashMap::default()
    };
    let wrapped_resolver = create_resolver(
        &path,
        resolution_opts,
        config,
        provided_iris,
        client,
        runtime,
    )?;

    let LockOutcome {
        lock,
        dependencies: _dependencies,
    } = match do_lock_local_editable(&path, wrapped_resolver) {
        Ok(lock_outcome) => lock_outcome,
        Err(LockProjectError::LockError(lock_error)) => {
            return Err(handle_lock_error(lock_error));
        }
        Err(err) => Err(err)?,
    };

    let canonical = lock.canonicalize();
    wrapfs::write(
        path.as_ref().join(DEFAULT_LOCKFILE_NAME),
        canonical.to_string(),
    )?;

    Ok(canonical)
}

pub fn create_resolver<P: AsRef<Path>>(
    path: &P,
    resolution_opts: ResolutionOptions,
    config: &Config,
    provided_iris: HashMap<String, Vec<InMemoryProject>>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<
    PriorityResolver<MemoryResolver<AcceptAll, InMemoryProject>, StandardResolver>,
    anyhow::Error,
> {
    let ResolutionOptions {
        index,
        default_index,
        no_index,
        include_std: _,
    } = resolution_opts;

    let cwd = wrapfs::current_dir()?;
    let local_env_path = path
        .as_ref()
        .join(sysand_core::env::local_directory::DEFAULT_ENV_NAME);

    let index_urls = if no_index {
        None
    } else {
        Some(config.index_urls(index, vec![DEFAULT_INDEX_URL.to_string()], default_index)?)
    };

    // TODO: add fn next to known_std_libs() to get this structure directly
    // it is created in most? all? places where `known_std_libs()` is used
    let mut memory_projects = HashMap::default();
    for (k, v) in provided_iris {
        memory_projects.insert(fluent_uri::Iri::parse(k).unwrap(), v);
    }

    let wrapped_resolver = PriorityResolver::new(
        MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: memory_projects,
        },
        standard_resolver(
            Some(cwd),
            if local_env_path.is_dir() {
                Some(local_env_path)
            } else {
                None
            },
            Some(client),
            index_urls,
            runtime,
        ),
    );
    Ok(wrapped_resolver)
}

pub fn handle_lock_error<PR: ProjectRead, RR: ResolveRead + std::fmt::Debug>(
    lock_error: LockError<PR, RR>,
) -> anyhow::Error {
    if let LockError::Solver(solver_error) = lock_error {
        match *solver_error.inner {
            pubgrub::PubGrubError::NoSolution(mut derivation_tree) => {
                derivation_tree.collapse_no_versions();

                anyhow!(
                    "failed to satisfy usage constraints:\n{}",
                    pubgrub::DefaultStringReporter::report(&derivation_tree)
                )
            }
            pubgrub::PubGrubError::ErrorRetrievingDependencies {
                package, source, ..
            } => match package {
                DependencyIdentifier::Requested(_) => {
                    // TODO: when can this happen?
                    // This should always indicate either I/O failure or a bug.
                    anyhow!()

                    match source {
                        InternalSolverError::Resolution(err) => {
                            anyhow!("resolution error: {}", err)
                        }
                        // InternalSolverError::InvalidProject => {
                        //     anyhow!("found invalid project during usage resolution")
                        // }
                        InternalSolverError::NotResolvable(iri) => {
                            anyhow!("unable to resolve usage `{}`", iri)
                        }
                        InternalSolverError::VersionNotAvailable(msg) => {
                            anyhow!("requested version unavailable: {msg}")
                        }
                    }
                    // anyhow!("unexpected internal error: {}", source)
                }
                DependencyIdentifier::Remote(iri) => {
                    anyhow!("failed to retrieve dependencies of usage `{}`", iri)
                }
            },
            pubgrub::PubGrubError::ErrorChoosingVersion { package, source } => match package {
                DependencyIdentifier::Requested(_) => {
                    // TODO: investigate when this can happen and give appropriate errors
                    // This should always indicate either I/O failure or a bug.

                    match source {
                        InternalSolverError::Resolution(err) => {
                            anyhow!("resolution error: {}", err)
                        }
                        // InternalSolverError::InvalidProject => {
                        //     anyhow!("found invalid project during usage resolution")
                        // }
                        InternalSolverError::NotResolvable(msg) => {
                            anyhow!("unable to resolve usage(s): {msg}")
                        }
                        InternalSolverError::VersionNotAvailable(msg) => {
                            anyhow!("requested version unavailable: {msg}")
                        }
                    }
                    // anyhow!("unexpected internal error: {}", source)
                }
                DependencyIdentifier::Remote(iri) => {
                    // anyhow!("unable to select version of usage `{iri}`: {source}")
                    match source {
                        InternalSolverError::Resolution(err) => {
                            anyhow!("resolution error during resolution of `{iri}`: {err}")
                        }
                        // InternalSolverError::InvalidProject => {
                        //     anyhow!("found invalid project during usage resolution")
                        // }
                        InternalSolverError::NotResolvable(msg) => {
                            anyhow!("unable to resolve usage `{iri}`: {msg}")
                        }
                        InternalSolverError::VersionNotAvailable(msg) => {
                            anyhow!("requested version of `{iri}` unavailable: {msg}")
                        }
                    }
                }
            },
            pubgrub::PubGrubError::ErrorInShouldCancel(err) => match err {
                InternalSolverError::Resolution(err) => {
                    anyhow!("resolution error: {}", err)
                }
                // InternalSolverError::InvalidProject => {
                //     anyhow!("found invalid project during usage resolution")
                // }
                InternalSolverError::NotResolvable(iri) => {
                    anyhow!("unable to resolve usage `{}`", iri)
                }
                InternalSolverError::VersionNotAvailable(msg) => {
                    anyhow!("requested version unavailable: {msg}")
                }
            },
        }
    } else {
        anyhow!("{lock_error}")
    }
}
