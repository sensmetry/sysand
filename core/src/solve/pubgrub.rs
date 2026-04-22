// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use pubgrub::{
    DefaultStringReporter, DependencyConstraints, DependencyProvider, Reporter, VersionSet,
};

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, hash_map::Entry},
    fmt::Write as _,
    fmt::{self, Display},
};

use thiserror::Error;

use crate::{
    model::{InterchangeProjectInfo, InterchangeProjectMetadataRaw, InterchangeProjectUsage},
    project::ProjectRead,
    resolve::ResolveRead,
};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum DependencyIdentifier {
    /// Dependencies that are to be resolved.
    Requested(Vec<InterchangeProjectUsage>),
    /// Found dependencies. Note that this does not mean that the
    /// required version was found, just that the IRI was resolved.
    Remote(fluent_uri::Iri<String>),
}

impl Display for DependencyIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DependencyIdentifier::Requested(_requested) => {
                write!(f, "requested project(s)")
                // if requested.len() == 1 {
                //     let req = &requested[0];

                //     write!(f, "requested project {}", req.resource)?;

                //     if let Some(vr) = &req.version_constraint {
                //         write!(f, " ({})", vr)?;
                //     }

                //     return write!(f, "");
                // }

                // write!(f, "requested projects [")?;
                // for (i, req) in requested.iter().enumerate() {
                //     if i > 0 {
                //         write!(f, ", ")?;
                //     }

                //     write!(f, "{}", req.resource)?;

                //     if let Some(vr) = &req.version_constraint {
                //         write!(f, " ({})", vr)?;
                //     }
                // }

                // write!(f, "]")
            }
            DependencyIdentifier::Remote(iri) => write!(f, "{}", iri),
        }
    }
}

// NOTE: Eq instance is not formally correct, but any set large
//       enough to observe the incorrectness would require a hash map
//       of at least about 10 exabyte.
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum DiscreteHashSet {
    Finite(HashSet<ProjectIndex>),
    CoFinite(HashSet<ProjectIndex>),
}

impl Display for DiscreteHashSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let elts = match self {
            DiscreteHashSet::Finite(hash_set) => {
                let elts: Vec<usize> = hash_set.iter().cloned().collect();

                if elts.is_empty() {
                    return write!(f, "no valid alternatives");
                } else if elts.len() == 1 {
                    return write!(f, "alternative nr {}", elts[0]);
                }

                write!(f, "one of alternatives ")?;
                elts
            }
            DiscreteHashSet::CoFinite(hash_set) => {
                let elts: Vec<usize> = hash_set.iter().cloned().collect();

                if elts.is_empty() {
                    return write!(f, "any alternative");
                } else if elts.len() == 1 {
                    return write!(f, "any alternative except nr {}", elts[0]);
                }

                write!(f, "any alternative except numbers ")?;
                elts
            }
        };

        for (i, v) in elts.iter().enumerate() {
            if i != 0 {
                write!(f, ", ")?;
            }

            if i == elts.len() - 1 {
                write!(f, "and ")?;
            }

            write!(f, "{}", v)?;
        }

        write!(f, ")")
    }
}

pub type ProjectIndex = usize;

impl VersionSet for DiscreteHashSet {
    type V = ProjectIndex;

    fn empty() -> Self {
        DiscreteHashSet::Finite(HashSet::new())
    }

    fn singleton(v: Self::V) -> Self {
        DiscreteHashSet::Finite(HashSet::from([v]))
    }

    fn complement(&self) -> Self {
        match self {
            DiscreteHashSet::Finite(hash_set) => Self::CoFinite(hash_set.clone()),
            DiscreteHashSet::CoFinite(hash_set) => Self::Finite(hash_set.clone()),
        }
    }

    fn intersection(&self, other: &Self) -> Self {
        match (self, other) {
            (DiscreteHashSet::Finite(hash_set), DiscreteHashSet::Finite(other_hash_set)) => {
                let intersection: HashSet<ProjectIndex> =
                    hash_set.intersection(other_hash_set).cloned().collect();

                DiscreteHashSet::Finite(intersection)
            }
            (DiscreteHashSet::Finite(hash_set), DiscreteHashSet::CoFinite(other_hash_set)) => {
                let difference: HashSet<ProjectIndex> =
                    hash_set.difference(other_hash_set).cloned().collect();

                DiscreteHashSet::Finite(difference)
            }
            (DiscreteHashSet::CoFinite(hash_set), DiscreteHashSet::Finite(other_hash_set)) => {
                let difference: HashSet<ProjectIndex> =
                    other_hash_set.difference(hash_set).cloned().collect();

                DiscreteHashSet::Finite(difference)
            }
            (DiscreteHashSet::CoFinite(hash_set), DiscreteHashSet::CoFinite(other_hash_set)) => {
                let union: HashSet<ProjectIndex> =
                    hash_set.union(other_hash_set).cloned().collect();

                DiscreteHashSet::CoFinite(union)
            }
        }
    }

    fn contains(&self, v: &Self::V) -> bool {
        match self {
            DiscreteHashSet::Finite(hash_set) => hash_set.contains(v),
            DiscreteHashSet::CoFinite(hash_set) => !hash_set.contains(v),
        }
    }
}

type ResolvedCandidates<ProjectStorage> = HashMap<
    fluent_uri::Iri<String>,
    Vec<(
        InterchangeProjectInfo,
        InterchangeProjectMetadataRaw,
        ProjectStorage,
    )>,
>;

pub struct ProjectSolver<R: ResolveRead> {
    // Internal RefCell, used in order to lazily populate the cache during resolution
    resolved_candidates: RefCell<ResolvedCandidates<R::ProjectStorage>>,
    // dependency_provider: OfflineDependencyProvider<DependencyIdentifier, DiscreteHashSet>,
    resolver: R,
}

/// Returned Vec will have `len >= 1`
fn resolve_candidates<R: ResolveRead>(
    resolver: &R,
    uri: &fluent_uri::Iri<String>,
    cache: &mut ResolvedCandidates<R::ProjectStorage>,
) -> Result<Vec<(InterchangeProjectInfo, InterchangeProjectMetadataRaw)>, InternalSolverError<R>> {
    let entry = cache.entry(uri.clone());

    match entry {
        Entry::Occupied(occupied_entry) => Ok(occupied_entry
            .get()
            .iter()
            .map(|(info, meta, _)| (info.clone(), meta.clone()))
            .collect()),
        Entry::Vacant(vacant_entry) => {
            let mut found = vec![];

            match resolver
                .resolve_read(uri)
                .map_err(InternalSolverError::Resolution)?
            {
                crate::resolve::ResolutionOutcome::UnsupportedIRIType(msg) => {
                    return Err(InternalSolverError::UnsupportedIriType(format!(
                        "unsupported IRI type of `{uri}`: {msg}"
                    )));
                }
                crate::resolve::ResolutionOutcome::Unresolvable(msg) => {
                    return Err(InternalSolverError::NotFound(uri.as_str().into(), msg));
                }
                crate::resolve::ResolutionOutcome::Resolved(alternatives) => {
                    for alternative in alternatives {
                        let project = match alternative {
                            Ok(project) => project,
                            Err(e) => {
                                log::debug!("candidate project for `{uri}` is error: {e}");
                                continue;
                            }
                        };

                        let (info, meta) = match project.get_project() {
                            Ok((Some(info), Some(meta))) => (info, meta),
                            Ok(incomplete) => {
                                log::debug!(
                                    "candidate project for `{uri}` failed to get info or meta: {incomplete:?}"
                                );
                                continue;
                            }
                            Err(e) => {
                                log::debug!(
                                    "candidate project for `{uri}` failed to get info and meta: {e}"
                                );
                                continue;
                            }
                        };

                        let validated_info: InterchangeProjectInfo = match info.try_into() {
                            Ok(i) => i,
                            Err(e) => {
                                log::debug!("candidate project for `{uri}` has invalid info: {e}");
                                continue;
                            }
                        };

                        found.push((validated_info, meta, project));
                    }
                    if found.is_empty() {
                        return Err(InternalSolverError::NoValidCandidates(uri.as_str().into()));
                    }
                }
            }

            let result: Vec<(InterchangeProjectInfo, InterchangeProjectMetadataRaw)> = found
                .iter()
                .map(|(info, meta, _)| (info.clone(), meta.clone()))
                .collect();

            vacant_entry.insert(found);

            Ok(result)
        }
    }
}

fn compute_deps<R: ResolveRead + fmt::Debug>(
    resolver: &R,
    usages: &Vec<InterchangeProjectUsage>,
    cache: &mut ResolvedCandidates<R::ProjectStorage>,
) -> Result<
    pubgrub::Dependencies<DependencyIdentifier, DiscreteHashSet, String>,
    InternalSolverError<R>,
> {
    let mut deps: Vec<(DependencyIdentifier, DiscreteHashSet)> = Vec::new();

    for usage in usages {
        if let Some(constraint) = &usage.version_constraint {
            let mut valid_candidates = HashSet::new();

            let mut found_versions = Vec::new();
            for (i, (candidate_info, _)) in resolve_candidates(resolver, &usage.resource, cache)?
                .iter()
                .enumerate()
            {
                found_versions.push(candidate_info.version.clone());
                if constraint.matches(&candidate_info.version) {
                    valid_candidates.insert(i);
                }
            }
            if valid_candidates.is_empty() {
                let mut versions = String::new();
                // `found_versions` must contain at least one element
                write!(versions, "`{}`", found_versions[0]).unwrap();
                for v in &found_versions[1..] {
                    write!(versions, ", `{}`", v).unwrap();
                }
                return Err(InternalSolverError::VersionNotAvailable(format!(
                    "project `{}`\n\
                    was found, but the requested version constraint `{}`\n\
                    was not satisfied by any of the found versions:\n\
                    {}",
                    usage.resource, constraint, versions
                )));
            }

            deps.push((
                DependencyIdentifier::Remote(usage.resource.clone()),
                DiscreteHashSet::Finite(valid_candidates),
            ));
        } else {
            // Check that the project can be found
            resolve_candidates(resolver, &usage.resource, cache)?;
            // TODO: reenable this when it's fixed to give better error messages
            // https://github.com/pubgrub-rs/pubgrub/pull/216
            // match resolve_candidates(resolver, &usage.resource, cache) {
            //     Ok(_) => (),
            //     Err(err) => return Ok(pubgrub::Dependencies::Unavailable(err.to_string())),
            // };

            deps.push((
                DependencyIdentifier::Remote(usage.resource.clone()),
                DiscreteHashSet::empty().complement(),
            ));
        }
    }

    // TODO: replace this with `from(deps)` when https://github.com/pubgrub-rs/pubgrub/pull/423
    // is merged and released
    let constraints = DependencyConstraints::from_iter(deps);
    Ok(pubgrub::Dependencies::Available(constraints))
}

#[derive(Debug)]
pub struct SolverError<R: ResolveRead + fmt::Debug + 'static> {
    pub inner: Box<pubgrub::PubGrubError<ProjectSolver<R>>>,
}

impl<R: ResolveRead + fmt::Debug + 'static> From<Box<pubgrub::PubGrubError<ProjectSolver<R>>>>
    for SolverError<R>
{
    fn from(mut value: Box<pubgrub::PubGrubError<ProjectSolver<R>>>) -> Self {
        if let pubgrub::PubGrubError::NoSolution(ref mut derivation_tree) = *value {
            derivation_tree.collapse_no_versions();
        }
        Self { inner: value }
    }
}

impl<R: ResolveRead + fmt::Debug + 'static> From<pubgrub::PubGrubError<ProjectSolver<R>>>
    for SolverError<R>
{
    fn from(value: pubgrub::PubGrubError<ProjectSolver<R>>) -> Self {
        Self::from(Box::new(value))
    }
}

impl<R: ResolveRead + fmt::Debug + 'static> Display for SolverError<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.as_ref() {
            pubgrub::PubGrubError::NoSolution(derivation_tree) => {
                writeln!(
                    f,
                    "failed to satisfy usage constraints:\n{}",
                    DefaultStringReporter::report(derivation_tree)
                )
            }
            pubgrub::PubGrubError::ErrorRetrievingDependencies {
                package, source, ..
            } => match package {
                DependencyIdentifier::Requested(_) => {
                    write!(f, "failed to retrieve project(s): {source}")
                }
                DependencyIdentifier::Remote(iri) => {
                    write!(f, "failed to retrieve usages of `{iri}`: {source}")
                }
            },
            pubgrub::PubGrubError::ErrorChoosingVersion { package, source } => match package {
                DependencyIdentifier::Requested(_) => {
                    // `fn choose_version()` is infallible in this path
                    unreachable!();
                }
                DependencyIdentifier::Remote(iri) => {
                    write!(f, "unable to select version of `{iri}`: {source}")
                }
            },
            pubgrub::PubGrubError::ErrorInShouldCancel(_) => {
                // ProjectSolver doesn't implement this and default impl does nothing
                unreachable!();
            }
        }
    }
}

impl<R: ResolveRead + fmt::Debug + 'static> std::error::Error for SolverError<R> {}

#[derive(Error, Debug)]
pub enum InternalSolverError<R: ResolveRead> {
    #[error("resolution error: {0}")]
    Resolution(R::Error),
    // #[error("invalid project requested")]
    // InvalidProject,
    /// Project not found by current resolver
    /// Value is the formatted error message
    #[error("project with IRI `{0}` not found: {1}")]
    NotFound(Box<str>, String),
    /// Project candidates were found, but none of them were
    /// valid.
    /// Value is the formatted error message
    #[error("no valid candidates found for project `{0}`")]
    NoValidCandidates(Box<str>),
    /// Project not found by current resolver
    /// Value is the formatted error message
    #[error("IRI is of type not supported by this resolver: {0}")]
    UnsupportedIriType(String),
    /// Project is found, but the requested version is not
    /// Value is the formatted error message
    #[error("requested version unavailable: {0}")]
    VersionNotAvailable(String),
}

impl<R: ResolveRead> ProjectSolver<R> {
    pub fn new(resolver: R) -> Self {
        ProjectSolver {
            resolved_candidates: RefCell::new(HashMap::new()),
            //dependency_provider: OfflineDependencyProvider::<DependencyIdentifier, DiscreteHashSet>::new(),
            resolver,
        }
    }

    //let mut map: RefMut<'_, _> = self.resolved_candidates.borrow_mut();
}

impl<R: ResolveRead + fmt::Debug + 'static> DependencyProvider for ProjectSolver<R> {
    type P = DependencyIdentifier;

    type V = ProjectIndex;

    type VS = DiscreteHashSet;

    type Priority = std::cmp::Reverse<usize>;

    type M = String;

    type Err = InternalSolverError<R>;

    fn prioritize(
        &self,
        _package: &Self::P,
        range: &Self::VS,
        _package_conflicts_counts: &pubgrub::PackageResolutionStatistics,
    ) -> Self::Priority {
        match range {
            DiscreteHashSet::Finite(hash_set) => std::cmp::Reverse(hash_set.len()),
            DiscreteHashSet::CoFinite(_) => std::cmp::Reverse(0),
        }
    }

    fn choose_version(
        &self,
        package: &Self::P,
        range: &Self::VS,
    ) -> Result<Option<Self::V>, Self::Err> {
        match range {
            DiscreteHashSet::Finite(hash_set) => {
                let res = hash_set.iter().min().cloned();
                log::debug!("choosing version for request ({:?})", res);
                Ok(res)
            }
            DiscreteHashSet::CoFinite(hash_set) => {
                match package {
                    DependencyIdentifier::Requested(_) => {
                        log::debug!("unknown version for request");
                        Ok(None)
                    }
                    DependencyIdentifier::Remote(iri) => {
                        let candidate_versions = resolve_candidates(
                            &self.resolver,
                            iri,
                            &mut self.resolved_candidates.borrow_mut(),
                        )?;
                        let mut versions_indexes: Vec<(usize, semver::Version)> =
                            candidate_versions
                                .into_iter()
                                .enumerate()
                                // Versions are usually returned in ascending order.
                                // Since we need them in descending order, sort will need
                                // to perform less work if the iterator is reversed
                                .rev()
                                .map(|(idx, el)| (idx, el.0.version))
                                .collect();
                        // Choose the highest version. We'll assume that version
                        // order is stable across multiple `resolve_candidates()`
                        // calls, as DiscreteHashSet does not save actual versions
                        versions_indexes.sort_unstable_by(|el1, el2| el2.1.cmp(&el1.1));
                        let mut found = None;
                        for (i, v) in versions_indexes.iter() {
                            if !hash_set.contains(i) {
                                found = Some(*i);
                                log::debug!("chose version for `{}`: {}", iri.as_str(), v);
                                break;
                            }
                        }
                        if found.is_none() {
                            log::debug!(
                                "no allowed versions for `{}`, considered: {:?}",
                                iri.as_str(),
                                versions_indexes
                            );
                        }

                        Ok(found)
                    }
                }
            }
        }
    }

    fn get_dependencies(
        &self,
        package: &Self::P,
        version: &Self::V,
    ) -> Result<pubgrub::Dependencies<Self::P, Self::VS, Self::M>, Self::Err> {
        match package {
            DependencyIdentifier::Requested(usages) => compute_deps(
                &self.resolver,
                usages,
                &mut self.resolved_candidates.borrow_mut(),
            ),
            DependencyIdentifier::Remote(iri) => {
                let info = {
                    let candidates = resolve_candidates(
                        &self.resolver,
                        iri,
                        &mut self.resolved_candidates.borrow_mut(),
                    )?;

                    if *version >= candidates.len() {
                        return Ok(pubgrub::Dependencies::Unavailable(format!(
                            "cannot resolve IRI `{}` to valid project",
                            iri
                        )));
                    } else {
                        candidates[*version].0.clone()
                    }
                };

                compute_deps(
                    &self.resolver,
                    &info.usage,
                    &mut self.resolved_candidates.borrow_mut(),
                )
            }
        }
    }
}

type Solution<ProjectStorage> = HashMap<
    Iri<String>,
    (
        InterchangeProjectInfo,
        InterchangeProjectMetadataRaw,
        ProjectStorage,
    ),
>;

pub fn solve<R: ResolveRead + fmt::Debug + 'static>(
    requested: Vec<InterchangeProjectUsage>,
    resolver: R,
) -> Result<Solution<R::ProjectStorage>, SolverError<R>> {
    let solver = ProjectSolver::new(resolver);

    let package = DependencyIdentifier::Requested(requested);

    let version: usize = 0;

    let solution = pubgrub::resolve(&solver, package, version)?;

    let mut map = solver.resolved_candidates.replace(HashMap::default());

    let mut result: HashMap<
        fluent_uri::Iri<String>,
        (
            InterchangeProjectInfo,
            InterchangeProjectMetadataRaw,
            <R as ResolveRead>::ProjectStorage,
        ),
        _,
    > = HashMap::default();

    for (k, idx) in solution {
        if let DependencyIdentifier::Remote(uri) = k {
            let mut extracted = map.remove(&uri).expect("internal solver error");

            result.insert(uri, extracted.swap_remove(idx));
        }
    }

    Ok(result)
}

#[cfg(test)]
#[path = "./pubgrub_tests.rs"]
mod tests;
