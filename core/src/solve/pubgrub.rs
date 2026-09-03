// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use camino::Utf8PathBuf;
use pubgrub::{
    DefaultStringReporter, DependencyConstraints, DependencyProvider, DerivationTree, External,
    Reporter as _, VersionSet,
};

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, hash_map::Entry},
    fmt::Write as _,
    fmt::{self, Display},
};

use thiserror::Error;

use crate::{
    model::{InterchangeProjectUsage, InterchangeProjectValidationError},
    project::{ProjectRead, utils::Identifier},
    resolve::{CoalescingUsage, ResolutionInfo, ResolutionOutcome, ResolveRead},
    utils::format_err,
};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum DependencyIdentifier {
    /// Dependencies that are to be resolved.
    Requested(Vec<CoalescingUsage>),
    /// Found dependencies. Note that this does not mean that the
    /// required version was found, just that the usage was resolved.
    Remote(CoalescingUsage),
}

impl Display for DependencyIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Requested(_requested) => {
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
            Self::Remote(iri) => write!(f, "{iri}"),
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
            Self::Finite(hash_set) => {
                let elts: Vec<usize> = hash_set.iter().copied().collect();

                if elts.is_empty() {
                    return write!(f, "no valid alternatives");
                } else if elts.len() == 1 {
                    return write!(f, "alternative nr {}", elts[0]);
                }

                write!(f, "one of alternatives ")?;
                elts
            }
            Self::CoFinite(hash_set) => {
                let elts: Vec<usize> = hash_set.iter().copied().collect();

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

            write!(f, "{v}")?;
        }

        write!(f, ")")
    }
}

/// pubgrub's "version" of a project: the position of a candidate in the
/// list `resolve_candidates` returns for its identifier. The list is
/// cached per identifier for the whole solve, so a position names the same
/// candidate every time it is seen. Positions are only ever assigned by
/// `numbered`; everything that turns a `DiscreteHashSet` back into
/// candidates goes through that numbering.
pub type ProjectIndex = usize;

/// Numbers `candidates` the way pubgrub refers to them.
fn numbered<T>(candidates: &[T]) -> impl Iterator<Item = (ProjectIndex, &T)> {
    candidates.iter().enumerate()
}

/// The set of `candidates` that a usage with `constraint` selects: those
/// whose version matches, or every candidate when there is no constraint.
fn selected_by<'a>(
    candidates: impl IntoIterator<Item = (ProjectIndex, &'a semver::Version)>,
    constraint: Option<&semver::VersionReq>,
) -> DiscreteHashSet {
    match constraint {
        Some(constraint) => DiscreteHashSet::Finite(
            candidates
                .into_iter()
                .filter(|(_, version)| constraint.matches(version))
                .map(|(index, _)| index)
                .collect(),
        ),
        None => DiscreteHashSet::empty().complement(),
    }
}

impl DiscreteHashSet {
    /// True when the set selects nothing.
    fn is_empty(&self) -> bool {
        match self {
            Self::Finite(hash_set) => hash_set.is_empty(),
            Self::CoFinite(_) => false,
        }
    }

    /// True when `self` and `other` select the same candidates out of
    /// `universe`. Unlike `==`, this does not care whether a set is stored
    /// as the candidates it contains or as those it excludes.
    fn selects_same(&self, other: &Self, universe: impl IntoIterator<Item = ProjectIndex>) -> bool {
        universe
            .into_iter()
            .all(|index| self.contains(&index) == other.contains(&index))
    }
}

impl VersionSet for DiscreteHashSet {
    type V = ProjectIndex;

    fn empty() -> Self {
        Self::Finite(HashSet::new())
    }

    fn singleton(v: Self::V) -> Self {
        Self::Finite(HashSet::from([v]))
    }

    fn complement(&self) -> Self {
        match self {
            Self::Finite(hash_set) => Self::CoFinite(hash_set.clone()),
            Self::CoFinite(hash_set) => Self::Finite(hash_set.clone()),
        }
    }

    fn intersection(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Finite(hash_set), Self::Finite(other_hash_set)) => {
                let intersection: HashSet<ProjectIndex> =
                    hash_set.intersection(other_hash_set).copied().collect();

                Self::Finite(intersection)
            }
            (Self::Finite(hash_set), Self::CoFinite(other_hash_set)) => {
                let difference: HashSet<ProjectIndex> =
                    hash_set.difference(other_hash_set).copied().collect();

                Self::Finite(difference)
            }
            (Self::CoFinite(hash_set), Self::Finite(other_hash_set)) => {
                let difference: HashSet<ProjectIndex> =
                    other_hash_set.difference(hash_set).copied().collect();

                Self::Finite(difference)
            }
            (Self::CoFinite(hash_set), Self::CoFinite(other_hash_set)) => {
                let union: HashSet<ProjectIndex> =
                    hash_set.union(other_hash_set).copied().collect();

                Self::CoFinite(union)
            }
        }
    }

    fn contains(&self, v: &Self::V) -> bool {
        match self {
            Self::Finite(hash_set) => hash_set.contains(v),
            Self::CoFinite(hash_set) => !hash_set.contains(v),
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
    usage: Vec<CoalescingUsage>,
}

pub struct ProjectSolver<R: ResolveRead> {
    // Internal RefCell, used in order to lazily populate the cache during resolution
    resolved_candidates: RefCell<CandidateMap<R::ProjectStorage>>,
    // dependency_provider: OfflineDependencyProvider<DependencyIdentifier, DiscreteHashSet>,
    resolver: R,
}

/// Returned Vec will have `len >= 1`
#[expect(clippy::result_large_err)]
fn resolve_candidates<R: ResolveRead>(
    resolver: &R,
    resolve: &CoalescingUsage,
    cache: &mut CandidateMap<R::ProjectStorage>,
) -> Result<Vec<CandidateSummary>, InternalSolverError<R>> {
    let entry = cache.entry(resolve.to_id());

    // TODO: decide on a resolution policy. Currently the cache is keyed by Identifier,
    // which may be encountered multiple times with different sources in the dependency
    // graph; only the first such encounter will be cached, all others will reuse the cached
    // one. SysML spec does not allow multiple instances of the same project to be present
    // in the dependency graph (AFAIK), so they should be coalesced to a single one (like is
    // currently done), but how exactly? Most specific (Directory > Resource)? Priority to
    // direct dependencies of the root project? Either way, the policy has to be explicitly
    // documented
    match entry {
        Entry::Occupied(occupied_entry) => Ok(occupied_entry
            .get()
            .iter()
            .map(|c| c.summary.clone())
            .collect()),
        Entry::Vacant(vacant_entry) => {
            let mut found = vec![];

            match resolver
                .resolve_read(resolve.usage())
                .map_err(InternalSolverError::Resolution)?
            {
                ResolutionOutcome::UnsupportedUsageType { reason } => {
                    return Err(InternalSolverError::UnsupportedUsageType {
                        usage: resolve.to_usage(),
                        reason,
                    });
                }
                ResolutionOutcome::Unresolvable { reason } => {
                    return Err(InternalSolverError::Unresolvable {
                        usage: resolve.to_usage(),
                        reason,
                    });
                }
                ResolutionOutcome::Resolved(alternatives) => {
                    // Treat `Resource` usages leniently, as they may produce invalid candidates that should
                    // not fail the whole resolution (e.g. both src and kpar variants for paths and http)
                    // Typed usages in contrast are required to be present and valid projects, so missing
                    // or invalid ones will fail resolution
                    let typed = resolve.usage().usage().is_typed();
                    for alternative in alternatives {
                        let project = match alternative {
                            Ok(project) => project,
                            Err(e) => {
                                if typed {
                                    return Err(InternalSolverError::ResolvedError {
                                        usage: resolve.to_usage(),
                                        source: e,
                                    });
                                } else {
                                    log::debug!(
                                        "candidate project for {resolve} is error: {}",
                                        format_err(e)
                                    );
                                    continue;
                                }
                            }
                        };

                        let version = match project.version() {
                            Ok(Some(version)) => match semver::Version::parse(&version) {
                                Ok(version) => version,
                                Err(e) => {
                                    if typed {
                                        return Err(InternalSolverError::InvalidResolvedVersion {
                                            usage: resolve.to_usage(),
                                            source: e,
                                        });
                                    } else {
                                        log::debug!(
                                            "candidate project for {resolve} has invalid version `{version}`: {}",
                                            format_err(e)
                                        );
                                        continue;
                                    }
                                }
                            },
                            Ok(None) => {
                                if typed {
                                    return Err(InternalSolverError::MissingVersion {
                                        usage: resolve.to_usage(),
                                    });
                                } else {
                                    log::debug!(
                                        "candidate project for {resolve} did not expose a version"
                                    );
                                    continue;
                                }
                            }
                            Err(e) => {
                                if typed {
                                    return Err(InternalSolverError::VersionObtain {
                                        usage: resolve.to_usage(),
                                        source: e,
                                    });
                                } else {
                                    log::debug!(
                                        "candidate project for {resolve} failed to get version: {}",
                                        format_err(e)
                                    );
                                    continue;
                                }
                            }
                        };

                        let usage = match project.usage() {
                            Ok(Some(usages)) => {
                                let validated: Result<Vec<InterchangeProjectUsage>, _> =
                                    usages.into_iter().map(|usage| usage.validate()).collect();
                                match validated {
                                    Ok(usage) => usage,
                                    Err(e) => {
                                        if typed {
                                            return Err(InternalSolverError::InvalidProject {
                                                usage: resolve.to_usage(),
                                                source: e,
                                            });
                                        } else {
                                            log::debug!(
                                                "candidate project for {resolve} has invalid usage: {}",
                                                format_err(e)
                                            );
                                            continue;
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                if typed {
                                    return Err(InternalSolverError::MissingUsage {
                                        usage: resolve.to_usage(),
                                    });
                                } else {
                                    log::debug!(
                                        "candidate project for {resolve} did not expose usages"
                                    );
                                    continue;
                                }
                            }
                            Err(e) => {
                                if typed {
                                    return Err(InternalSolverError::UsageObtain {
                                        usage: resolve.to_usage(),
                                        source: e,
                                    });
                                } else {
                                    log::debug!(
                                        "candidate project for {resolve} failed to get usages: {}",
                                        format_err(e)
                                    );
                                    continue;
                                }
                            }
                        };
                        let relative_root =
                            project.project_root().map(camino::Utf8Path::to_path_buf);
                        let usage = usage
                            .into_iter()
                            .map(|u| CoalescingUsage::new_usage(u, relative_root.clone()))
                            .collect();

                        found.push(Candidate {
                            summary: CandidateSummary { version, usage },
                            project,
                        });
                    }
                    if found.is_empty() {
                        return Err(InternalSolverError::NoValidCandidates(resolve.to_usage()));
                    }
                }
                ResolutionOutcome::NotFound { reason } => {
                    return Err(InternalSolverError::NotFound(resolve.to_usage(), reason));
                }
            }

            let result: Vec<CandidateSummary> = found.iter().map(|c| c.summary.clone()).collect();

            vacant_entry.insert(found);

            Ok(result)
        }
    }
}

#[expect(clippy::result_large_err)]
fn compute_deps<R: ResolveRead + fmt::Debug>(
    resolver: &R,
    usages: &[CoalescingUsage],
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

        match usage.usage().usage() {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint,
            } => {
                let selected = selected_by(
                    numbered(&candidates).map(|(index, c)| (index, &c.version)),
                    version_constraint.as_ref(),
                );
                if let Some(constraint) = version_constraint
                    && selected.is_empty()
                {
                    return Err(InternalSolverError::VersionNotAvailable {
                        resource: resource.to_string(),
                        constraint: constraint.clone(),
                        found: candidates.iter().map(|c| c.version.clone()).collect(),
                    });
                }

                deps.push((DependencyIdentifier::Remote(usage.to_owned()), selected));
            }
            InterchangeProjectUsage::Directory { .. }
            | InterchangeProjectUsage::KparPath { .. } => {
                // Usually `candidates` will contain a single candidate for these types,
                // but e.g. environment may have multiple project versions, and will
                // return all of them based on the identifier
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

/// The versions a `NoVersions` conflict lists: ascending, each once.
/// Candidates come in resolver order, and two sources offering the same
/// release put that version in the list twice, so sort (by semver, not
/// by string) before removing neighbours.
fn version_strings<'a>(versions: impl IntoIterator<Item = &'a semver::Version>) -> Vec<String> {
    let mut versions: Vec<&semver::Version> = versions.into_iter().collect();
    versions.sort_unstable();
    versions.dedup();
    versions.into_iter().map(ToString::to_string).collect()
}

/// Render `found` as `` `v1`, `v2` `` — the shape the CLI has always shown.
fn format_found_versions(found: &[semver::Version]) -> String {
    let mut versions = String::new();
    for (i, v) in found.iter().enumerate() {
        if i > 0 {
            versions.push_str(", ");
        }
        write!(versions, "`{v}`").unwrap();
    }
    versions
}

/// What the solver learnt about one candidate before it failed: enough to
/// name versions behind `DiscreteHashSet` indices and to recover the
/// constraint a dependent put on one of its usages.
#[derive(Debug, Clone)]
pub struct CandidateSnapshot {
    /// The position pubgrub knows this candidate by.
    pub index: ProjectIndex,
    pub version: semver::Version,
    pub usages: Vec<CoalescingUsage>,
}

/// One machine-readable participant in a failed solve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolveConflict {
    /// `iri` is constrained to `constraint` by `required_by` (`None` when
    /// the requirement comes from the root request).
    Constraint {
        iri: String,
        constraint: String,
        required_by: Option<String>,
    },
    /// `iri` exists but none of `found` satisfies `constraint`. `found` is
    /// sorted ascending with duplicates removed.
    NoVersions {
        iri: String,
        constraint: String,
        found: Vec<String>,
        required_by: Option<String>,
    },
    /// `iri` could not be retrieved at all.
    NotFound { iri: String, reason: String },
}

#[derive(Debug)]
pub struct SolverError<R: ResolveRead + fmt::Debug + 'static> {
    pub inner: Box<pubgrub::PubGrubError<ProjectSolver<R>>>,
    /// Candidates per identifier, captured by `solve()` on failure. pubgrub
    /// speaks in candidate indices; this is what turns them back into
    /// versions and constraints.
    pub candidates: HashMap<Identifier, Vec<CandidateSnapshot>>,
}

impl<R: ResolveRead + fmt::Debug + 'static> From<Box<pubgrub::PubGrubError<ProjectSolver<R>>>>
    for SolverError<R>
{
    fn from(mut value: Box<pubgrub::PubGrubError<ProjectSolver<R>>>) -> Self {
        if let pubgrub::PubGrubError::NoSolution(ref mut derivation_tree) = *value {
            derivation_tree.collapse_no_versions();
        }
        Self {
            inner: value,
            candidates: HashMap::new(),
        }
    }
}

impl<R: ResolveRead + fmt::Debug + 'static> From<pubgrub::PubGrubError<ProjectSolver<R>>>
    for SolverError<R>
{
    fn from(value: pubgrub::PubGrubError<ProjectSolver<R>>) -> Self {
        Self::from(Box::new(value))
    }
}

/// The version constraint a usage puts on its resource, `*` when none (or
/// when the usage is not a `Resource` usage).
fn constraint_of(usage: &CoalescingUsage) -> String {
    match usage.usage().usage() {
        InterchangeProjectUsage::Resource {
            version_constraint: Some(constraint),
            ..
        } => constraint.to_string(),
        _ => "*".to_owned(),
    }
}

/// The identifier a solver package stands for; `None` for the root request.
fn id_of(package: &DependencyIdentifier) -> Option<&CoalescingUsage> {
    match package {
        DependencyIdentifier::Requested(_) => None,
        DependencyIdentifier::Remote(usage) => Some(usage),
    }
}

impl<R: ResolveRead + fmt::Debug + 'static> SolverError<R> {
    /// Which way the solve failed: `no_solution` (the constraints
    /// contradict each other), `retrieval` (a project or version could not
    /// be obtained), or `choosing_version`.
    pub fn kind(&self) -> &'static str {
        match self.inner.as_ref() {
            pubgrub::PubGrubError::NoSolution(_) => "no_solution",
            pubgrub::PubGrubError::ErrorRetrievingDependencies { .. } => "retrieval",
            pubgrub::PubGrubError::ErrorChoosingVersion { .. }
            | pubgrub::PubGrubError::ErrorInShouldCancel(_) => "choosing_version",
        }
    }

    /// The resolver error behind a retrieval failure, if that is what this
    /// is: the caller may want to classify a transport or authentication
    /// problem differently from a genuine solve failure.
    pub fn resolution_error(&self) -> Option<&R::Error> {
        match self.inner.as_ref() {
            pubgrub::PubGrubError::ErrorRetrievingDependencies {
                source: InternalSolverError::Resolution(err),
                ..
            }
            | pubgrub::PubGrubError::ErrorRetrievingDependencies {
                source: InternalSolverError::ResolvedError { source: err, .. },
                ..
            } => Some(err),
            _ => None,
        }
    }

    /// Flattened, machine-readable view of the failure, in encounter order
    /// without duplicates. Empty only for transport failures (see
    /// [`Self::resolution_error`]) and version-choice failures.
    pub fn conflicts(&self) -> Vec<SolveConflict> {
        let mut conflicts = Vec::new();
        match self.inner.as_ref() {
            pubgrub::PubGrubError::NoSolution(derivation_tree) => {
                self.walk(derivation_tree, &mut conflicts);
            }
            pubgrub::PubGrubError::ErrorRetrievingDependencies {
                package, source, ..
            } => {
                let required_by = id_of(package).map(|u| u.id().to_string());
                match source {
                    InternalSolverError::VersionNotAvailable {
                        resource,
                        constraint,
                        found,
                    } => conflicts.push(SolveConflict::NoVersions {
                        iri: resource.clone(),
                        constraint: constraint.to_string(),
                        found: version_strings(found),
                        required_by,
                    }),
                    InternalSolverError::Resolution(_)
                    | InternalSolverError::ResolvedError { .. } => {}
                    InternalSolverError::NotFound(usage, _)
                    | InternalSolverError::NoValidCandidates(usage)
                    | InternalSolverError::UnsupportedUsageType { usage, .. }
                    | InternalSolverError::Unresolvable { usage, .. }
                    | InternalSolverError::InvalidProject { usage, .. }
                    | InternalSolverError::MissingVersion { usage }
                    | InternalSolverError::MissingUsage { usage }
                    | InternalSolverError::InvalidResolvedVersion { usage, .. }
                    | InternalSolverError::VersionObtain { usage, .. }
                    | InternalSolverError::UsageObtain { usage, .. } => {
                        conflicts.push(SolveConflict::NotFound {
                            iri: usage.id().to_string(),
                            reason: format_err(source),
                        });
                    }
                }
            }
            pubgrub::PubGrubError::ErrorChoosingVersion { .. }
            | pubgrub::PubGrubError::ErrorInShouldCancel(_) => {}
        }
        conflicts
    }

    fn walk(
        &self,
        tree: &DerivationTree<DependencyIdentifier, DiscreteHashSet, String>,
        out: &mut Vec<SolveConflict>,
    ) {
        match tree {
            DerivationTree::Derived(derived) => {
                self.walk(&derived.cause1, out);
                self.walk(&derived.cause2, out);
            }
            DerivationTree::External(external) => {
                let conflict = match external {
                    External::NotRoot(..) => None,
                    External::FromDependencyOf(
                        dependent,
                        dependent_set,
                        dependency,
                        dependency_set,
                    ) => {
                        // pubgrub interns packages by identifier, so the
                        // `dependency` package may carry another party's usage,
                        // so only the identifier of it is correct to use here.
                        // Because `dependency_set` is currently
                        // `DiscreteHashSet` which does not include version
                        // constraints in a user-readable form, version
                        // constraints are instead extracted from the usage list
                        // of `dependent`.
                        // FIXME: make `dependency_set` usable directly
                        id_of(dependency).map(|dependency| {
                            let iri = dependency.id().to_string();
                            let (required_by, constraint) = match dependent {
                                DependencyIdentifier::Requested(usages) => (
                                    None,
                                    self.select_usage(
                                        usages.iter().filter(|u| u.id() == dependency.id()),
                                        dependency.id(),
                                        dependency_set,
                                    ),
                                ),
                                DependencyIdentifier::Remote(usage) => (
                                    Some(usage.id().to_string()),
                                    self.select_usage(
                                        self.candidates_in(usage.id(), dependent_set)
                                            .into_iter()
                                            .flat_map(|c| c.usages.iter())
                                            .filter(|u| u.id() == dependency.id()),
                                        dependency.id(),
                                        dependency_set,
                                    ),
                                ),
                            };
                            SolveConflict::Constraint {
                                iri,
                                constraint: constraint_of(constraint.unwrap_or(dependency)),
                                required_by,
                            }
                        })
                    }
                    External::NoVersions(package, set) => {
                        id_of(package).map(|usage| SolveConflict::NoVersions {
                            iri: usage.id().to_string(),
                            constraint: constraint_of(usage),
                            found: self.versions_of(usage.id(), set),
                            required_by: None,
                        })
                    }
                    External::Custom(package, _, reason) => {
                        id_of(package).map(|usage| SolveConflict::NotFound {
                            iri: usage.id().to_string(),
                            reason: reason.clone(),
                        })
                    }
                };
                if let Some(conflict) = conflict
                    && !out.contains(&conflict)
                {
                    out.push(conflict);
                }
            }
        }
    }

    /// The candidates of `id` selected by `set`.
    fn candidates_in(&self, id: &Identifier, set: &DiscreteHashSet) -> Vec<&CandidateSnapshot> {
        self.candidates
            .get(id)
            .into_iter()
            .flatten()
            .filter(|candidate| set.contains(&candidate.index))
            .collect()
    }

    /// Among several usages of `dependency` declared by one dependent (a
    /// project may list the same resource twice), the one whose constraint
    /// selects exactly the candidates in `dependency_set`; otherwise the
    /// first.
    fn select_usage<'a>(
        &self,
        mut usages: impl Iterator<Item = &'a CoalescingUsage>,
        dependency: &Identifier,
        dependency_set: &DiscreteHashSet,
    ) -> Option<&'a CoalescingUsage> {
        let first = usages.next()?;
        let candidates: &[CandidateSnapshot] =
            self.candidates.get(dependency).map_or(&[], Vec::as_slice);
        let selects_set = |usage: &CoalescingUsage| match usage.usage().usage() {
            InterchangeProjectUsage::Resource {
                version_constraint: Some(constraint),
                ..
            } => selected_by(
                candidates.iter().map(|c| (c.index, &c.version)),
                Some(constraint),
            )
            .selects_same(dependency_set, candidates.iter().map(|c| c.index)),
            _ => false,
        };
        if selects_set(first) {
            return Some(first);
        }
        Some(usages.find(|u| selects_set(u)).unwrap_or(first))
    }

    fn versions_of(&self, id: &Identifier, set: &DiscreteHashSet) -> Vec<String> {
        version_strings(self.candidates_in(id, set).into_iter().map(|c| &c.version))
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

/// Error of `R`'s resolved project storage. Also hides the nested projection
/// from `derive(Debug)`, which would otherwise emit an unsatisfiable
/// `R::ProjectStorage: Debug` bound.
type StorageError<R> = <<R as ResolveRead>::ProjectStorage as ProjectRead>::Error;

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
    /// Project not found by current resolver
    /// Value is the formatted error message
    #[error("usage {usage} is of type not supported by this resolver: {reason}")]
    UnsupportedUsageType {
        usage: ResolutionInfo,
        reason: String,
    },
    /// Project is found, but the requested version is not
    #[error(
        "requested version unavailable: project `{resource}`\n\
         was found, but the requested version constraint `{constraint}`\n\
         was not satisfied by any of the found versions:\n\
         {}",
        format_found_versions(found)
    )]
    VersionNotAvailable {
        resource: String,
        constraint: semver::VersionReq,
        /// Every version the resolver offered, in resolver order.
        found: Vec<semver::Version>,
    },
    /// Resolution failed due to an invalid usage that is in principle supported
    #[error("usage {usage} is not resolvable: {reason}")]
    Unresolvable {
        usage: ResolutionInfo,
        reason: String,
    },
    #[error("usage {usage} resolved to an error")]
    ResolvedError {
        usage: ResolutionInfo,
        source: R::Error,
    },
    #[error("usage {usage} resolved to an error")]
    InvalidProject {
        usage: ResolutionInfo,
        source: InterchangeProjectValidationError,
    },
    #[error("usage {usage} resolved to a project that does not expose its version")]
    MissingVersion { usage: ResolutionInfo },
    #[error("usage {usage} resolved to a project that does not expose its usages")]
    MissingUsage { usage: ResolutionInfo },
    #[error("usage {usage} resolved to a project that has an invalid version")]
    InvalidResolvedVersion {
        usage: ResolutionInfo,
        source: semver::Error,
    },
    #[error("failed to obtain version of resolved usage {usage}")]
    VersionObtain {
        usage: ResolutionInfo,
        source: StorageError<R>,
    },
    #[error("failed to obtain usage of resolved usage {usage}")]
    UsageObtain {
        usage: ResolutionInfo,
        source: StorageError<R>,
    },
}

impl<R: ResolveRead> ProjectSolver<R> {
    pub fn new(resolver: R) -> Self {
        Self {
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
                let res = hash_set.iter().min().copied();
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
                        let mut versions_indexes: Vec<(ProjectIndex, semver::Version)> =
                            numbered(&candidate_versions)
                                .map(|(index, c)| (index, c.version.clone()))
                                .collect();
                        // Choose the highest version. Positions are stable
                        // across `resolve_candidates()` calls, see `ProjectIndex`.
                        versions_indexes.sort_unstable_by(|el1, el2| el2.1.cmp(&el1.1));
                        let mut found = None;
                        for (i, v) in &versions_indexes {
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
                            "cannot resolve IRI `{iri}` to valid project"
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
        .map(|u| CoalescingUsage::new_usage(u, base_path.clone()))
        .collect();
    let package = DependencyIdentifier::Requested(requested);

    let version: usize = 0;

    let solution = match pubgrub::resolve(&solver, package, version) {
        Ok(solution) => solution,
        Err(err) => {
            let mut err = SolverError::from(err);
            err.candidates = solver
                .resolved_candidates
                .take()
                .into_iter()
                .map(|(id, candidates)| {
                    (
                        id,
                        numbered(&candidates)
                            .map(|(index, c)| CandidateSnapshot {
                                index,
                                version: c.summary.version.clone(),
                                usages: c.summary.usage.clone(),
                            })
                            .collect(),
                    )
                })
                .collect();
            return Err(err);
        }
    };

    let mut map = solver.resolved_candidates.take();

    let mut result = HashMap::default();

    for (k, idx) in solution {
        if let DependencyIdentifier::Remote(usage) = k {
            let (_, id) = usage.into_parts();
            let mut extracted = map.remove(&id).expect("internal solver error");

            result.insert(id, extracted.swap_remove(idx).project);
        }
    }

    Ok(result)
}

#[cfg(test)]
#[path = "./pubgrub_tests.rs"]
mod tests;
