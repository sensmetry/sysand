// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use fluent_uri::Iri;
use pubgrub::{DependencyProvider, VersionSet};

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, hash_map::Entry},
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
    Requested(Vec<InterchangeProjectUsage>),
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
                crate::resolve::ResolutionOutcome::UnsupportedIRIType(_) => {
                    return Err(InternalSolverError::NotResolvable(uri.clone()));
                }
                crate::resolve::ResolutionOutcome::Unresolvable(_) => {
                    return Err(InternalSolverError::NotResolvable(uri.clone()));
                }
                crate::resolve::ResolutionOutcome::Resolved(alternatives) => {
                    for alternative in alternatives {
                        let Ok(project) = alternative else {
                            continue;
                        };

                        let Ok((Some(info), Some(meta))) = project.get_project() else {
                            continue;
                        };

                        let Ok(validated_info): Result<InterchangeProjectInfo, _> = info.try_into()
                        else {
                            continue;
                        };

                        found.push((validated_info, meta, project));
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

fn compute_deps<R: ResolveRead>(
    resolver: &R,
    usages: &Vec<InterchangeProjectUsage>,
    cache: &mut ResolvedCandidates<R::ProjectStorage>,
) -> Result<
    pubgrub::Dependencies<DependencyIdentifier, DiscreteHashSet, String>,
    InternalSolverError<R>,
> {
    let mut depmap: HashMap<DependencyIdentifier, DiscreteHashSet, _> = pubgrub::Map::default();

    for usage in usages {
        if let Some(constraint) = &usage.version_constraint {
            let mut valid_candidates = HashSet::new();

            for (i, (candidate_info, _)) in resolve_candidates(resolver, &usage.resource, cache)?
                .iter()
                .enumerate()
            {
                if constraint.matches(&candidate_info.version) {
                    valid_candidates.insert(i);
                }
            }

            depmap.insert(
                DependencyIdentifier::Remote(usage.resource.clone()),
                DiscreteHashSet::Finite(valid_candidates),
            );
        } else {
            depmap.insert(
                DependencyIdentifier::Remote(usage.resource.clone()),
                DiscreteHashSet::empty().complement(),
            );
        }
    }

    Ok(pubgrub::Dependencies::Available(depmap))
}

#[derive(Error, Debug)]
#[error("{inner}")]
pub struct SolverError<R: ResolveRead + fmt::Debug + 'static> {
    #[from]
    pub inner: Box<pubgrub::PubGrubError<ProjectSolver<R>>>,
}

impl<R: ResolveRead + fmt::Debug + 'static> From<pubgrub::PubGrubError<ProjectSolver<R>>>
    for SolverError<R>
{
    fn from(value: pubgrub::PubGrubError<ProjectSolver<R>>) -> Self {
        Self {
            inner: Box::new(value),
        }
    }
}

#[derive(Error, Debug)]
pub enum InternalSolverError<R: ResolveRead> {
    #[error(transparent)]
    Resolution(R::Error),
    // #[error("invalid project requested")]
    // InvalidProject,
    #[error("not resolvable: {0}")]
    NotResolvable(fluent_uri::Iri<String>),
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
        let result = match range {
            DiscreteHashSet::Finite(hash_set) => Ok(hash_set.iter().min().cloned()),
            DiscreteHashSet::CoFinite(hash_set) => {
                let mut found = None;

                let max = match package {
                    DependencyIdentifier::Requested(_) => 0,
                    DependencyIdentifier::Remote(iri) => resolve_candidates(
                        &self.resolver,
                        iri,
                        &mut self.resolved_candidates.borrow_mut(),
                    )?
                    .len(),
                };

                for i in 0..max {
                    if !hash_set.contains(&i) {
                        found = Some(i);
                        break;
                    }
                }

                Ok(found)
            }
        };

        match package {
            DependencyIdentifier::Requested(_) => {
                log::debug!("Choosing version for request ({:?})", result)
            }
            DependencyIdentifier::Remote(iri) => {
                log::debug!("Choosing version for {} ({:?})", iri.as_str(), result)
            }
        }

        result
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
                            "Cannot resolve {} to valid project",
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

    let solution = pubgrub::resolve(&solver, package.clone(), version)?;

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
                description: None,
                version: version.to_string(),
                license: None,
                maintainer: vec![],
                website: None,
                topic: vec![],
                usage: usage
                    .into_iter()
                    .map(|(d, dv)| InterchangeProjectUsageRaw {
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
    ) -> EnvResolver<MemoryStorageEnvironment> {
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
            vec![InterchangeProjectUsage {
                resource: fluent_uri::Iri::parse("urn:kpar:test_version_selection".to_string())
                    .unwrap(),
                version_constraint: Some(semver::VersionReq::parse(">=2.0.0")?),
            }],
            resolver,
        )?;

        assert_eq!(solution.len(), 1);

        let (install_info, _, _) = solution
            .get(&Iri::parse("urn:kpar:test_version_selection".to_string()).unwrap())
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
                InterchangeProjectUsage {
                    resource: fluent_uri::Iri::parse(
                        "urn:kpar:test_diamond_selection_a".to_string(),
                    )
                    .unwrap(),
                    version_constraint: Some(semver::VersionReq::parse(">=0.1.0")?),
                },
                InterchangeProjectUsage {
                    resource: fluent_uri::Iri::parse(
                        "urn:kpar:test_diamond_selection_b".to_string(),
                    )
                    .unwrap(),
                    version_constraint: None,
                },
            ],
            resolver,
        )?;

        assert_eq!(solution.len(), 3);

        let (install_info_a, _, _) = solution
            .get(&Iri::parse("urn:kpar:test_diamond_selection_a".to_string()).unwrap())
            .unwrap();
        assert_eq!(install_info_a.version, semver::Version::new(1, 0, 1));

        let (install_info_b, _, _) = solution
            .get(&Iri::parse("urn:kpar:test_diamond_selection_b".to_string()).unwrap())
            .unwrap();
        assert_eq!(install_info_b.version, semver::Version::new(1, 0, 2));

        let (install_info_a, _, _) = solution
            .get(&Iri::parse("urn:kpar:test_diamond_selection_c".to_string()).unwrap())
            .unwrap();
        assert_eq!(install_info_a.version, semver::Version::new(2, 0, 3));

        Ok(())
    }
}
