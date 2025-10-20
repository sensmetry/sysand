// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::env::current_dir;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use pubgrub::Reporter as _;
use sysand_core::commands;
use sysand_core::commands::lock::{
    DEFAULT_LOCKFILE_NAME, LockError, LockOutcome, LockProjectError,
};

use sysand_core::project::ProjectRead;
use sysand_core::project::memory::InMemoryProject;
use sysand_core::project::utils::wrapfs;
use sysand_core::resolve::ResolveRead;
use sysand_core::resolve::memory::{AcceptAll, MemoryResolver};
use sysand_core::resolve::priority::PriorityResolver;
use sysand_core::resolve::standard::standard_resolver;
use sysand_core::solve::pubgrub::{DependencyIdentifier, InternalSolverError};

pub fn command_lock<P: AsRef<Path>, S: AsRef<str>>(
    path: P,
    client: reqwest_middleware::ClientWithMiddleware,
    index_base_urls: Option<Vec<S>>,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<sysand_core::lock::Lock> {
    let wrapped_resolver = create_resolver(&path, client, index_base_urls, runtime, provided_iris)?;

    let LockOutcome {
        lock,
        dependencies: _dependencies,
        inputs: _inputs,
    } = match commands::lock::do_lock_local_editable(&path, wrapped_resolver) {
        Ok(lock_outcome) => lock_outcome,
        Err(LockProjectError::LockError(lock_error)) => {
            return Err(handle_lock_error(lock_error));
        }
        Err(err) => Err(err)?,
    };

    let canonical = lock.canonicalize();
    wrapfs::write(
        Path::new(path.as_ref()).join(DEFAULT_LOCKFILE_NAME),
        canonical.to_string(),
    )?;

    Ok(canonical)
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
                    match source {
                        InternalSolverError::Resolution(err) => {
                            anyhow!("resolution error: {:?}", err)
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
                    // anyhow!("unexpected internal error: {:?}", source)
                }
                DependencyIdentifier::Remote(iri) => {
                    anyhow!("failed to retrieve (transitive) usages of usage `{}`", iri)
                }
            },
            pubgrub::PubGrubError::ErrorChoosingVersion { package, source } => match package {
                DependencyIdentifier::Requested(_) => {
                    match source {
                        InternalSolverError::Resolution(err) => {
                            anyhow!("resolution error: {:?}", err)
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
                    // anyhow!("unexpected internal error: {:?}", source)
                }
                DependencyIdentifier::Remote(iri) => {
                    anyhow!("unable to select version of usage `{iri}`: {source}")
                }
            },
            pubgrub::PubGrubError::ErrorInShouldCancel(err) => match err {
                InternalSolverError::Resolution(err) => {
                    anyhow!("resolution error: {:?}", err)
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

pub fn create_resolver<P: AsRef<Path>, S: AsRef<str>>(
    path: P,
    client: reqwest_middleware::ClientWithMiddleware,
    index_base_urls: Option<Vec<S>>,
    runtime: Arc<tokio::runtime::Runtime>,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
) -> Result<
    PriorityResolver<
        MemoryResolver<AcceptAll, InMemoryProject>,
        sysand_core::resolve::standard::StandardResolver,
    >,
> {
    let cwd = current_dir().ok();
    let local_env_path =
        Path::new(path.as_ref()).join(sysand_core::env::local_directory::DEFAULT_ENV_NAME);

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
            index_base_urls
                .map(|xs| xs.iter().map(|x| url::Url::parse(x.as_ref())).collect())
                .transpose()?,
            runtime,
        ),
    );
    Ok(wrapped_resolver)
}
