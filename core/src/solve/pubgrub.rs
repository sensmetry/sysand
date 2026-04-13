// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::{Utf8Path, Utf8PathBuf};
use pubgrub::{DefaultStringReporter, DependencyProvider, Reporter, VersionSet};

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, hash_map::Entry},
    fmt::Write as _,
    fmt::{self, Display},
};

use thiserror::Error;

use crate::{
    model::{InterchangeProjectInfo, InterchangeProjectMetadataRaw, InterchangeProjectUsage},
    project::{ProjectRead, utils::Identifier},
    resolve::{ResolutionOutcome, ResolveRead},
};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum DependencyIdentifier {
    /// Dependencies that are to be resolved.
    Requested(Vec<InterchangeProjectUsage>, Option<Utf8PathBuf>),
    /// Found dependencies. Note that this does not mean that the
    /// required version was found, just that the IRI was resolved.
    Remote(InterchangeProjectUsage, Option<Utf8PathBuf>),
}

impl Display for DependencyIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DependencyIdentifier::Requested(_requested, _base_path) => {
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
            DependencyIdentifier::Remote(iri, ..) => write!(f, "{}", iri),
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
    Identifier,
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

// easyfind4
// Resolving path deps:
// - absolute paths: no special treatment, will be resolved for all local/remote projects
// - relative paths:
//   - if dep of local, resolve, taking as base the path of the using project
//   - if dep of git, resolve, taking as base the path of the locally cloned repo,
//     BUT RESTRICTING IT TO THE REPO (using file resolver's limiting functionality)

/// Returned Vec will have `len >= 1`
fn resolve_candidates<R: ResolveRead>(
    resolver: &R,
    usage: &InterchangeProjectUsage,
    base_path: Option<impl AsRef<Utf8Path>>,
    cache: &mut ResolvedCandidates<R::ProjectStorage>,
) -> Result<Vec<(InterchangeProjectInfo, InterchangeProjectMetadataRaw)>, InternalSolverError<R>> {
    let identifier = Identifier::from_interchange_usage(&usage);
    let entry = cache.entry(identifier);

    match entry {
        Entry::Occupied(occupied_entry) => Ok(occupied_entry
            .get()
            .iter()
            .map(|(info, meta, _)| (info.clone(), meta.clone()))
            .collect()),
        Entry::Vacant(vacant_entry) => {
            let mut found = vec![];

            match resolver
                .resolve_read(usage, base_path)
                .map_err(InternalSolverError::Resolution)?
            {
                ResolutionOutcome::UnsupportedUsageType { usage, reason } => {
                    return Err(InternalSolverError::UnsupportedUsageType { usage, reason });
                }
                ResolutionOutcome::NotFound(usage, reason) => {
                    return Err(InternalSolverError::NotFound(usage, reason));
                }
                ResolutionOutcome::Resolved(alternatives) => {
                    for alternative in alternatives {
                        let project = match alternative {
                            Ok(project) => project,
                            Err(e) => {
                                log::debug!("candidate project for {usage} is error: {e}");
                                continue;
                            }
                        };

                        let (info, meta) = match project.get_project() {
                            Ok((Some(info), Some(meta))) => (info, meta),
                            Ok(incomplete) => {
                                log::debug!(
                                    "candidate project for {usage} failed to get info or meta: {incomplete:?}"
                                );
                                continue;
                            }
                            Err(e) => {
                                log::debug!(
                                    "candidate project for {usage} failed to get info and meta: {e}"
                                );
                                continue;
                            }
                        };

                        let validated_info: InterchangeProjectInfo = match info.try_into() {
                            Ok(i) => i,
                            Err(e) => {
                                log::debug!("candidate project for {usage} has invalid info: {e}");
                                continue;
                            }
                        };

                        found.push((validated_info, meta, project));
                    }
                    if found.is_empty() {
                        return Err(InternalSolverError::NoValidCandidates(usage.to_owned()));
                    }
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    return Err(InternalSolverError::Unresolvable(msg));
                }
                ResolutionOutcome::InvalidUsage(..) => unreachable!(),
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
    base_path: Option<impl AsRef<Utf8Path>>,
    cache: &mut ResolvedCandidates<R::ProjectStorage>,
) -> Result<
    pubgrub::Dependencies<DependencyIdentifier, DiscreteHashSet, String>,
    InternalSolverError<R>,
> {
    let mut depmap: HashMap<DependencyIdentifier, DiscreteHashSet, _> = pubgrub::Map::default();

    for usage in usages {
        let mut valid_candidates = HashSet::new();

        // let id = Identifier::from_interchange_usage(usage);
        // let mut new_base_path = None;
        let mut found_versions = Vec::new();
        for (i, (candidate_info, _)) in resolve_candidates(resolver, usage, base_path, cache)?
            .iter()
            .enumerate()
        {
            found_versions.push(candidate_info.version.clone());
            if usage.version_satisfies_req(&candidate_info.version) {
                valid_candidates.insert(i);
                // Project must exist in cache if it's resolved
                // let pr = &cache.get(&id).unwrap()[i].2;
                // FIXME: this is an ugly hack that may resolve deps incorrectly if
                // the candidate that we actually use to resolve deps is different from the
                // first one that returns a base path
                // if new_base_path.is_none()
                // && let Some(bp) = pr.base_path_for_usage_resolver()
                // {
                // new_base_path = Some(bp)
                // }
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
                "project {usage}\n\
                    was found, but the requested version constraint\n\
                    was not satisfied by any of the found versions:\n\
                    {versions}",
            )));
        }

        let sources = depmap.insert(
            DependencyIdentifier::Remote(
                usage.to_owned(),
                base_path.map(|p| p.as_ref().to_owned()),
            ),
            DiscreteHashSet::Finite(valid_candidates),
        );
    }

    Ok(pubgrub::Dependencies::Available(depmap))
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
                DependencyIdentifier::Requested(..) => {
                    write!(f, "failed to retrieve project(s): {source}")
                }
                DependencyIdentifier::Remote(usage, ..) => {
                    // TODO: better error message here, publisher/name should be sufficient?
                    write!(f, "failed to retrieve usages of {usage}: {source}")
                }
            },
            pubgrub::PubGrubError::ErrorChoosingVersion { package, source } => match package {
                DependencyIdentifier::Requested(..) => {
                    // `fn choose_version()` is infallible in this path
                    unreachable!();
                }
                DependencyIdentifier::Remote(usage, _) => {
                    write!(f, "unable to select version of {usage}: {source}")
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
    #[error("project {0} not found: {1}")]
    NotFound(InterchangeProjectUsage, String),
    /// Project candidates were found, but none of them were
    /// valid.
    /// Value is the formatted error message
    #[error("no valid candidates found for project {0}")]
    NoValidCandidates(InterchangeProjectUsage),
    /// Project not found by current resolver
    /// Value is the formatted error message
    #[error("usage {usage} is of type not supported by this resolver: {reason}")]
    UnsupportedUsageType {
        usage: InterchangeProjectUsage,
        reason: String,
    },
    /// Project is found, but the requested version is not
    /// Value is the formatted error message
    #[error("requested version unavailable: {0}")]
    VersionNotAvailable(String),
    /// Resolution failed due to an invalid usage that is in principle supported
    #[error("usage is not resolvable: {0}")]
    Unresolvable(String),
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
                    DependencyIdentifier::Requested(..) => {
                        log::debug!("unknown version for request");
                        Ok(None)
                    }
                    DependencyIdentifier::Remote(usage, base_path) => {
                        let candidate_versions = resolve_candidates(
                            &self.resolver,
                            usage,
                            base_path.as_ref(),
                            &mut self.resolved_candidates.borrow_mut(),
                        )?;
                        let mut versions_indexes: Vec<(usize, semver::Version)> =
                            candidate_versions
                                .into_iter()
                                .enumerate()
                                // Versions are usually returned in ascending order from registry/env.
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
                                log::debug!("chose version for {usage}: {v}");
                                break;
                            }
                        }
                        if found.is_none() {
                            log::debug!(
                                "no allowed versions for {usage}, considered: {versions_indexes:?}"
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
            DependencyIdentifier::Requested(usages, base_path) => compute_deps(
                &self.resolver,
                usages,
                base_path.as_ref(),
                &mut self.resolved_candidates.borrow_mut(),
            ),
            DependencyIdentifier::Remote(usage, base_path) => {
                let identifier = Identifier::from_interchange_usage(usage);
                let (info, usage_resolve_path) = {
                    let candidates = resolve_candidates(
                        &self.resolver,
                        usage,
                        base_path.as_ref(),
                        &mut self.resolved_candidates.borrow_mut(),
                    )?;

                    if *version >= candidates.len() {
                        return Ok(pubgrub::Dependencies::Unavailable(format!(
                            "cannot resolve {usage} to valid project"
                        )));
                    } else {
                        // TODO: find a better solution than abusing cache like this
                        let cached_project_candidates =
                            &self.resolved_candidates.borrow()[&identifier];
                        (
                            candidates[*version].0.clone(),
                            cached_project_candidates[*version]
                                .2
                                .base_path_for_usage_resolver()
                                .to_owned()
                                .map(|p| p.to_owned()),
                        )
                    }
                };

                compute_deps(
                    &self.resolver,
                    &info.usage,
                    usage_resolve_path,
                    &mut self.resolved_candidates.borrow_mut(),
                )
            }
        }
    }
}

type Solution<ProjectStorage> = HashMap<
    Identifier,
    (
        InterchangeProjectInfo,
        InterchangeProjectMetadataRaw,
        ProjectStorage,
    ),
>;

pub fn solve<R: ResolveRead + fmt::Debug + 'static>(
    requested: Vec<InterchangeProjectUsage>,
    base_path: Option<Utf8PathBuf>,
    resolver: R,
) -> Result<Solution<R::ProjectStorage>, SolverError<R>> {
    let solver = ProjectSolver::new(resolver);

    let package = DependencyIdentifier::Requested(requested, base_path);

    let version: usize = 0;

    let solution = pubgrub::resolve(&solver, package, version)?;

    let mut map = solver.resolved_candidates.replace(HashMap::default());

    let mut result: HashMap<
        Identifier,
        (
            InterchangeProjectInfo,
            InterchangeProjectMetadataRaw,
            <R as ResolveRead>::ProjectStorage,
        ),
        _,
    > = HashMap::default();

    for (k, idx) in solution {
        if let DependencyIdentifier::Remote(usage, _base_path) = k {
            let identifier = Identifier::from_interchange_usage(&usage);
            let mut extracted = map.remove(&identifier).expect("internal solver error");

            result.insert(identifier, extracted.swap_remove(idx));
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use fluent_uri::Iri;
    use indexmap::IndexMap;

    use crate::{
        env::memory::MemoryStorageEnvironment,
        model::{
            InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsage,
            InterchangeProjectUsageRaw,
        },
        project::memory::InMemoryProject,
        resolve::env::EnvResolver,
    };

    fn trivial_memory_project(
        name: &str,
        version: &str,
        usage: Vec<(&str, Option<&str>)>,
    ) -> InMemoryProject {
        InMemoryProject {
            info: Some(InterchangeProjectInfoRaw {
                name: name.to_string(),
                publisher: None,
                description: None,
                version: version.to_string(),
                license: None,
                maintainer: vec![],
                website: None,
                topic: vec![],
                usage: usage
                    .into_iter()
                    .map(|(d, dv)| InterchangeProjectUsageRaw::Resource {
                        resource: d.to_string(),
                        version_constraint: dv.map(|x| x.to_string()),
                    })
                    .collect(),
            }),
            meta: Some(InterchangeProjectMetadataRaw {
                index: IndexMap::default(),
                created: "123".to_string(),
                metamodel: None,
                includes_derived: None,
                includes_implied: None,
                checksum: Some(IndexMap::default()),
            }),
            files: HashMap::default(),
            nominal_sources: vec![],
        }
    }

    fn simple_resolver_environment(
        structure: &[(&str, &[InMemoryProject])],
    ) -> EnvResolver<MemoryStorageEnvironment<InMemoryProject>> {
        EnvResolver {
            env: MemoryStorageEnvironment {
                projects: structure
                    .iter()
                    .map(|(x, projs)| {
                        (
                            x.to_string(),
                            projs
                                .iter()
                                .map(|proj| (proj.info.clone().unwrap().version, proj.clone()))
                                .collect(),
                        )
                    })
                    .collect(),
            },
        }
    }

    #[test]
    fn test_trivial_resolution() -> Result<(), Box<dyn std::error::Error>> {
        let resolver = simple_resolver_environment(&[]);

        let solution = super::solve(vec![], resolver)?;

        assert!(solution.is_empty());

        Ok(())
    }

    #[test]
    fn test_version_selection() -> Result<(), Box<dyn std::error::Error>> {
        let project_v1 = trivial_memory_project("test_version_selection", "1.0.1", vec![]);
        let project_v2 = trivial_memory_project("test_version_selection", "2.0.1", vec![]);

        let resolver = simple_resolver_environment(&[(
            "urn:kpar:test_version_selection",
            &[project_v1, project_v2],
        )]);

        let solution = super::solve(
            vec![InterchangeProjectUsage::Resource {
                resource: fluent_uri::Iri::parse("urn:kpar:test_version_selection")?.into(),
                version_constraint: Some(semver::VersionReq::parse(">=2.0.0")?),
            }],
            resolver,
        )?;

        assert_eq!(solution.len(), 1);

        let (install_info, _, _) = solution
            .get(Iri::parse("urn:kpar:test_version_selection")?.into())
            .unwrap();

        assert_eq!(install_info.version, semver::Version::new(2, 0, 1));

        Ok(())
    }

    #[test]
    fn test_version_constraint_default() -> Result<(), Box<dyn std::error::Error>> {
        // `semver` by default prepends `^` if a version requirement does not
        // have a comparator. This is not documented, but is also extremely
        // unlikely to change, as it's the behavior relied on by cargo
        let v_no_caret = semver::VersionReq::parse("2.0.0")?;
        let v_caret = semver::VersionReq::parse("^2.0.0")?;
        assert_eq!(v_no_caret, v_caret);

        Ok(())
    }

    #[test]
    fn test_diamond_selection() -> Result<(), Box<dyn std::error::Error>> {
        let project_a_v1 = trivial_memory_project(
            "test_diamond_selection_a",
            "1.0.1",
            vec![("urn:kpar:test_diamond_selection_c", Some(">=2.0.0"))],
        );
        let project_b_v1 = trivial_memory_project(
            "test_diamond_selection_b",
            "1.0.2",
            vec![("urn:kpar:test_diamond_selection_c", Some("<3.0.0"))],
        );

        let project_c_v1 = trivial_memory_project("test_diamond_selection_c", "1.0.3", vec![]);
        let project_c_v2 = trivial_memory_project("test_diamond_selection_c", "2.0.3", vec![]);
        let project_c_v3 = trivial_memory_project("test_diamond_selection_c", "3.0.3", vec![]);

        let resolver = simple_resolver_environment(&[
            ("urn:kpar:test_diamond_selection_a", &[project_a_v1]),
            ("urn:kpar:test_diamond_selection_b", &[project_b_v1]),
            (
                "urn:kpar:test_diamond_selection_c",
                &[project_c_v1, project_c_v2, project_c_v3],
            ),
        ]);

        let solution = super::solve(
            vec![
                InterchangeProjectUsage::Resource {
                    resource: fluent_uri::Iri::parse("urn:kpar:test_diamond_selection_a")?.into(),
                    version_constraint: Some(semver::VersionReq::parse(">=0.1.0")?),
                },
                InterchangeProjectUsage::Resource {
                    resource: fluent_uri::Iri::parse("urn:kpar:test_diamond_selection_b")?.into(),
                    version_constraint: None,
                },
            ],
            resolver,
        )?;

        assert_eq!(solution.len(), 3);

        let (install_info_a, _, _) = solution
            .get(Iri::parse("urn:kpar:test_diamond_selection_a")?.into())
            .unwrap();
        assert_eq!(install_info_a.version, semver::Version::new(1, 0, 1));

        let (install_info_b, _, _) = solution
            .get(Iri::parse("urn:kpar:test_diamond_selection_b")?.into())
            .unwrap();
        assert_eq!(install_info_b.version, semver::Version::new(1, 0, 2));

        let (install_info_a, _, _) = solution
            .get(Iri::parse("urn:kpar:test_diamond_selection_c")?.into())
            .unwrap();
        assert_eq!(install_info_a.version, semver::Version::new(2, 0, 3));

        Ok(())
    }
}
