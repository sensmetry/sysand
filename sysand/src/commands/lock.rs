// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::{env::current_dir, fs, path::Path};

use anyhow::{Result, bail};
use pubgrub::Reporter as _;
use reqwest::blocking::Client;
use sysand_core::commands::lock::{DEFAULT_LOCKFILE_NAME, LockOutcome};

use sysand_core::project::memory::InMemoryProject;
use sysand_core::resolve::memory::{AcceptAll, MemoryResolver};
use sysand_core::resolve::priority::PriorityResolver;
use sysand_core::resolve::standard::standard_resolver;

pub fn command_lock<P: AsRef<Path>, S: AsRef<str>>(
    path: P,
    client: Client,
    index_base_urls: Option<Vec<S>>,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
) -> Result<()> {
    let cwd = current_dir().ok();

    let local_env_path = std::path::Path::new(path.as_ref())
        .join(sysand_core::env::local_directory::DEFAULT_ENV_NAME);

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
        ),
    );

    let LockOutcome { lock, .. } =
        match sysand_core::commands::lock::do_lock_local_editable(&path, wrapped_resolver) {
            Ok(lock_outcome) => lock_outcome,
            Err(sysand_core::commands::lock::LockProjectError::LockError(lock_error)) => {
                if let sysand_core::commands::lock::LockError::SolverError(solver_error) =
                    lock_error
                {
                    match solver_error.inner {
                        pubgrub::PubGrubError::NoSolution(mut derivation_tree) => {
                            derivation_tree.collapse_no_versions();
                            bail!(
                                "Failed to satisfy usage constraints:\n{}",
                                pubgrub::DefaultStringReporter::report(&derivation_tree)
                            );
                        }
                        pubgrub::PubGrubError::ErrorRetrievingDependencies {
                            package,
                            source,
                            ..
                        } => match package {
                            sysand_core::solve::pubgrub::DependencyIdentifier::Requested(_) => {
                                bail!("Unexpected internal error: {:?}", source)
                            }
                            sysand_core::solve::pubgrub::DependencyIdentifier::Remote(iri) => {
                                bail!("Failed to retrieve (transitive) usages of usage {}", iri)
                            }
                        },
                        pubgrub::PubGrubError::ErrorChoosingVersion { package, source } => {
                            match package {
                                sysand_core::solve::pubgrub::DependencyIdentifier::Requested(_) => {
                                    bail!("Unxpected internal error: {:?}", source)
                                }
                                sysand_core::solve::pubgrub::DependencyIdentifier::Remote(iri) => {
                                    bail!("Unable to select version of usage {}", iri)
                                }
                            }
                        }
                        pubgrub::PubGrubError::ErrorInShouldCancel(err) => match err {
                            sysand_core::solve::pubgrub::InternalSolverError::ResolutionError(
                                err,
                            ) => {
                                bail! {"Resolution error {:?}", err}
                            }
                            sysand_core::solve::pubgrub::InternalSolverError::InvalidProject => {
                                bail!("Found invalid project during usage resolution")
                            }
                            sysand_core::solve::pubgrub::InternalSolverError::NotResolvable(
                                iri,
                            ) => {
                                bail!("Unable to resolve usage {}", iri)
                            }
                        },
                    }
                }
                Err(lock_error)?
            }
            Err(err) => Err(err)?,
        };

    fs::write(
        std::path::Path::new(path.as_ref()).join(DEFAULT_LOCKFILE_NAME),
        lock.to_toml().to_string(),
    )?;

    Ok(())
}
