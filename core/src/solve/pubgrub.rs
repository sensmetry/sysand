// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use camino::Utf8PathBuf;
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
    model::InterchangeProjectUsage,
    project::{ProjectRead, utils::Identifier},
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead},
    utils::format_err,
};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum DependencyIdentifier {
    /// Dependencies that are to be resolved.
    Requested(Vec<ResolutionInfo>),
    /// Found dependencies. Note that this does not mean that the
    /// required version was found, just that the usage was resolved.
    Remote(ResolutionInfo),
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

type CandidateMap<ProjectStorage> = HashMap<Identifier, Vec<Candidate<ProjectStorage>>>;

/// One resolved alternative for a given IRI: the summary the solver scores
/// against and the `ProjectStorage` we hand back at extraction time.
#[derive(Clone, Debug)]
struct Candidate<ProjectStorage> {
    summary: CandidateSummary,
    project: ProjectStorage,
}

/// The fields of a candidate project that the solver needs:
/// - `version` (for range matching)
/// - `usage` (for recursive dependency discovery)
#[derive(Clone, Debug)]
struct CandidateSummary {
    version: semver::Version,
    usage: Vec<ResolutionInfo>,
}

pub struct ProjectSolver<R: ResolveRead> {
    // Internal RefCell, used in order to lazily populate the cache during resolution
    resolved_candidates: RefCell<CandidateMap<R::ProjectStorage>>,
    // dependency_provider: OfflineDependencyProvider<DependencyIdentifier, DiscreteHashSet>,
    resolver: R,
}

/// Returned Vec will have `len >= 1`
fn resolve_candidates<R: ResolveRead>(
    resolver: &R,
    resolve: &ResolutionInfo,
    cache: &mut CandidateMap<R::ProjectStorage>,
) -> Result<Vec<CandidateSummary>, InternalSolverError<R>> {
    let entry = cache.entry(resolve.id());

    match entry {
        Entry::Occupied(occupied_entry) => Ok(occupied_entry
            .get()
            .iter()
            .map(|c| c.summary.clone())
            .collect()),
        Entry::Vacant(vacant_entry) => {
            let mut found = vec![];

            match resolver
                .resolve_read(resolve)
                .map_err(InternalSolverError::Resolution)?
            {
                ResolutionOutcome::UnsupportedUsageType { reason } => {
                    return Err(InternalSolverError::UnsupportedUsageType {
                        usage: resolve.to_owned(),
                        reason,
                    });
                }
                ResolutionOutcome::Unresolvable { reason } => {
                    return Err(InternalSolverError::Unresolvable {
                        usage: resolve.to_owned(),
                        reason,
                    });
                }
                ResolutionOutcome::Resolved(alternatives) => {
                    for alternative in alternatives {
                        let project = match alternative {
                            Ok(project) => project,
                            Err(e) => {
                                log::debug!(
                                    "candidate project for {resolve} is error: {}",
                                    format_err(e)
                                );
                                continue;
                            }
                        };

                        let version = match project.version() {
                            Ok(Some(version)) => match semver::Version::parse(&version) {
                                Ok(version) => version,
                                Err(e) => {
                                    log::debug!(
                                        "candidate project for {resolve} has invalid version `{version}`: {}",
                                        format_err(e)
                                    );
                                    continue;
                                }
                            },
                            Ok(None) => {
                                log::debug!(
                                    "candidate project for {resolve} did not expose a version"
                                );
                                continue;
                            }
                            Err(e) => {
                                log::debug!(
                                    "candidate project for {resolve} failed to get version: {}",
                                    format_err(e)
                                );
                                continue;
                            }
                        };

                        let usage = match project.usage() {
                            Ok(Some(usages)) => {
                                let validated: Result<Vec<InterchangeProjectUsage>, _> =
                                    usages.into_iter().map(|usage| usage.validate()).collect();
                                match validated {
                                    Ok(usage) => usage,
                                    Err(e) => {
                                        log::debug!(
                                            "candidate project for {resolve} has invalid usage: {}",
                                            format_err(e)
                                        );
                                        continue;
                                    }
                                }
                            }
                            Ok(None) => {
                                log::debug!(
                                    "candidate project for {resolve} did not expose usages"
                                );
                                continue;
                            }
                            Err(e) => {
                                log::debug!(
                                    "candidate project for {resolve} failed to get usages: {}",
                                    format_err(e)
                                );
                                continue;
                            }
                        };
                        let relative_root = project.project_root().map(|p| p.to_path_buf());
                        let usage = usage
                            .into_iter()
                            .map(|u| ResolutionInfo::new(u, relative_root.to_owned()))
                            .collect();

                        found.push(Candidate {
                            summary: CandidateSummary { version, usage },
                            project,
                        });
                    }
                    if found.is_empty() {
                        return Err(InternalSolverError::NoValidCandidates(resolve.to_owned()));
                    }
                }
                ResolutionOutcome::NotFound { reason } => {
                    return Err(InternalSolverError::NotFound(resolve.to_owned(), reason));
                }
            }

            let result: Vec<CandidateSummary> = found.iter().map(|c| c.summary.clone()).collect();

            vacant_entry.insert(found);

            Ok(result)
        }
    }
}

fn compute_deps<R: ResolveRead + fmt::Debug>(
    resolver: &R,
    usages: &[ResolutionInfo],
    cache: &mut CandidateMap<R::ProjectStorage>,
) -> Result<
    pubgrub::Dependencies<DependencyIdentifier, DiscreteHashSet, String>,
    InternalSolverError<R>,
> {
    let mut deps: Vec<(DependencyIdentifier, DiscreteHashSet)> = Vec::new();

    for usage in usages {
        let candidates = resolve_candidates(resolver, usage, cache)?;
        // TODO: reenable this when it's fixed to give better error messages
        // https://github.com/pubgrub-rs/pubgrub/pull/216
        // match resolve_candidates(resolver, &usage.resource, cache) {
        //     Ok(_) => (),
        //     Err(err) => return Ok(pubgrub::Dependencies::Unavailable(format_err(err))),
        // };

        match &usage.usage() {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint,
            } => {
                if let Some(constraint) = version_constraint {
                    let mut valid_candidates = HashSet::new();

                    let mut found_versions = Vec::new();
                    for (i, candidate_info) in candidates.iter().enumerate() {
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
                            "project `{resource}`\n\
                            was found, but the requested version constraint `{constraint}`\n\
                            was not satisfied by any of the found versions:\n\
                            {versions}"
                        )));
                    }

                    deps.push((
                        DependencyIdentifier::Remote(usage.to_owned()),
                        DiscreteHashSet::Finite(valid_candidates),
                    ));
                } else {
                    deps.push((
                        DependencyIdentifier::Remote(usage.to_owned()),
                        DiscreteHashSet::empty().complement(),
                    ));
                }
            }
            InterchangeProjectUsage::Directory {
                dir: _,
                publisher: _,
                name: _,
            } => {
                // TODO: verify publisher and name match expected
                assert_eq!(candidates.len(), 1);
                deps.push((
                    DependencyIdentifier::Remote(usage.to_owned()),
                    DiscreteHashSet::empty().complement(),
                ));
            }
        }
    }

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
    /// Project not found by current resolver
    /// Value is the formatted error message
    #[error("project {0} not found: {1}")]
    NotFound(ResolutionInfo, String),
    /// Project candidates were found, but none of them were
    /// valid.
    /// Value is the formatted error message
    #[error("no valid candidates found for project {0}")]
    NoValidCandidates(ResolutionInfo),
    // #[error("project at `{absolute_path}` exists, but instead of the expected\n\
    //     publisher `{expected_publisher}` and name `{expected_name}`,\n\
    //     it has a publisher {} and name `{name}`", if let Some(p)=publisher {format!("`{p}`")} else {String::new()})]
    // DifferentProject {
    //     absolute_path: Utf8PathBuf,
    //     expected_publisher: String,
    //     expected_name: String,
    //     publisher: Option<String>,
    //     name: String,
    // },
    /// Project not found by current resolver
    /// Value is the formatted error message
    #[error("usage {usage} is of type not supported by this resolver: {reason}")]
    UnsupportedUsageType {
        usage: ResolutionInfo,
        reason: String,
    },
    /// Project is found, but the requested version is not
    /// Value is the formatted error message
    #[error("requested version unavailable: {0}")]
    VersionNotAvailable(String),
    /// Resolution failed due to an invalid usage that is in principle supported
    #[error("usage {usage} is not resolvable: {reason}")]
    Unresolvable {
        usage: ResolutionInfo,
        reason: String,
    },
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
                log::debug!("choosing version for request ({res:?})");
                Ok(res)
            }
            DiscreteHashSet::CoFinite(hash_set) => {
                match package {
                    DependencyIdentifier::Requested(_) => {
                        log::debug!("unknown version for request");
                        Ok(None)
                    }
                    DependencyIdentifier::Remote(usage) => {
                        let candidate_versions = resolve_candidates(
                            &self.resolver,
                            usage,
                            &mut self.resolved_candidates.borrow_mut(),
                        )?;
                        let mut versions_indexes: Vec<(usize, semver::Version)> =
                            candidate_versions
                                .into_iter()
                                .enumerate()
                                .map(|(idx, el)| (idx, el.version))
                                .collect();
                        // Choose the highest version. We'll assume that version
                        // order is stable across multiple `resolve_candidates()`
                        // calls, as DiscreteHashSet does not save actual versions
                        versions_indexes.sort_unstable_by(|el1, el2| el2.1.cmp(&el1.1));
                        let mut found = None;
                        for (i, v) in versions_indexes.iter() {
                            if !hash_set.contains(i) {
                                found = Some(*i);
                                log::debug!("chose version for {usage}: {v}");
                                break;
                            }
                        }
                        if found.is_none() {
                            log::debug!(
                                "no allowed versions for {usage}, considered: {versions_indexes:?}",
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
                        candidates[*version].clone()
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

type Solution<ProjectStorage> = HashMap<Identifier, ProjectStorage>;

pub fn solve<R: ResolveRead + fmt::Debug + 'static>(
    requested: Vec<InterchangeProjectUsage>,
    base_path: Option<Utf8PathBuf>,
    resolver: R,
) -> Result<Solution<R::ProjectStorage>, SolverError<R>> {
    let solver = ProjectSolver::new(resolver);

    let requested = requested
        .into_iter()
        .map(|u| ResolutionInfo::new(u, base_path.clone()))
        .collect();
    let package = DependencyIdentifier::Requested(requested);

    let version: usize = 0;

    let solution = pubgrub::resolve(&solver, package, version)?;

    let mut map = solver.resolved_candidates.take();

    let mut result = HashMap::default();

    for (k, idx) in solution {
        if let DependencyIdentifier::Remote(usage) = k {
            let id = usage.id();
            let mut extracted = map.remove(&id).expect("internal solver error");

            result.insert(id, extracted.swap_remove(idx).project);
        }
    }

    Ok(result)
}

#[cfg(test)]
#[path = "./pubgrub_tests.rs"]
mod tests;
