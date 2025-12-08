// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};
use pubgrub::Reporter as _;

use sysand_core::{
    commands::lock::{
        DEFAULT_LOCKFILE_NAME, LockError, LockOutcome, LockProjectError, do_lock_local_editable,
    },
    config::Config,
    project::utils::wrapfs,
    resolve::{
        memory::{AcceptAll, MemoryResolver},
        priority::PriorityResolver,
        standard::standard_resolver,
    },
    solve::pubgrub::{DependencyIdentifier, InternalSolverError},
    stdlib::known_std_libs,
};

use crate::{DEFAULT_INDEX_URL, cli::ResolutionOptions};

/// Generate a lockfile for project at `path`.
/// `path` must be relative.
pub fn command_lock<P: AsRef<Path>>(
    path: P,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    assert!(path.as_ref().is_relative(), "{}", path.as_ref().display());
    let ResolutionOptions {
        index,
        default_index,
        no_index,
        include_std,
    } = resolution_opts;

    let cwd = wrapfs::current_dir().ok();

    let local_env_path = path
        .as_ref()
        .join(sysand_core::env::local_directory::DEFAULT_ENV_NAME);

    let index_urls = if no_index {
        None
    } else {
        Some(config.index_urls(index, vec![DEFAULT_INDEX_URL.to_string()], default_index)?)
    };

    let provided_iris = if !include_std {
        known_std_libs()
    } else {
        HashMap::default()
    };

    let mut memory_projects = HashMap::default();

    for (k, v) in provided_iris {
        memory_projects.insert(fluent_uri::Iri::parse(k.clone()).unwrap(), v.to_vec());
    }

    let wrapped_resolver = PriorityResolver::new(
        MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: memory_projects,
        },
        standard_resolver(
            cwd,
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

    let LockOutcome {
        lock,
        dependencies: _dependencies,
        inputs: _inputs,
    } = match do_lock_local_editable(&path, wrapped_resolver) {
        Ok(lock_outcome) => lock_outcome,
        Err(LockProjectError::LockError(lock_error)) => {
            if let LockError::Solver(solver_error) = lock_error {
                match *solver_error.inner {
                    pubgrub::PubGrubError::NoSolution(mut derivation_tree) => {
                        derivation_tree.collapse_no_versions();
                        bail!(
                            "Failed to satisfy usage constraints:\n{}",
                            pubgrub::DefaultStringReporter::report(&derivation_tree)
                        );
                    }
                    pubgrub::PubGrubError::ErrorRetrievingDependencies {
                        package, source, ..
                    } => match package {
                        DependencyIdentifier::Requested(_) => {
                            bail!("Unexpected internal error: {:?}", source)
                        }
                        DependencyIdentifier::Remote(iri) => {
                            bail!("Failed to retrieve (transitive) usages of usage {}", iri)
                        }
                    },
                    pubgrub::PubGrubError::ErrorChoosingVersion { package, source } => {
                        match package {
                            DependencyIdentifier::Requested(_) => {
                                bail!("Unxpected internal error: {:?}", source)
                            }
                            DependencyIdentifier::Remote(iri) => {
                                bail!("Unable to select version of usage {}", iri)
                            }
                        }
                    }
                    pubgrub::PubGrubError::ErrorInShouldCancel(err) => match err {
                        InternalSolverError::Resolution(err) => {
                            bail! {"Resolution error: {:?}", err}
                        }
                        // InternalSolverError::InvalidProject => {
                        //     bail!("Found invalid project during usage resolution")
                        // }
                        InternalSolverError::NotResolvable(iri) => {
                            bail!("Unable to resolve usage '{}'", iri)
                        }
                    },
                }
            }
            Err(lock_error)?
        }
        Err(err) => Err(err)?,
    };

    wrapfs::write(
        path.as_ref().join(DEFAULT_LOCKFILE_NAME),
        lock.canonicalize().to_string(),
    )?;

    Ok(())
}
