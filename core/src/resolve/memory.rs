// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, convert::Infallible};

use camino::Utf8Path;
use fluent_uri::{Iri, component::Scheme};

use crate::{
    model::InterchangeProjectUsage,
    project::{ProjectRead, utils::Identifier},
    resolve::{ResolutionOutcome, ResolveRead},
};

#[derive(Debug)]
pub struct MemoryResolver<Predicate, ProjectStorage: Clone> {
    pub iri_predicate: Predicate,
    pub projects: HashMap<Iri<String>, Vec<ProjectStorage>>,
}

impl<Project: ProjectRead + Clone> FromIterator<(Iri<String>, Vec<Project>)>
    for MemoryResolver<AcceptAll, Project>
{
    fn from_iter<T: IntoIterator<Item = (Iri<String>, Vec<Project>)>>(iter: T) -> Self {
        Self {
            iri_predicate: AcceptAll {},
            projects: HashMap::from_iter(iter),
        }
    }
}

impl<Project: ProjectRead + Clone, const N: usize> From<[(Iri<String>, Vec<Project>); N]>
    for MemoryResolver<AcceptAll, Project>
{
    fn from(value: [(Iri<String>, Vec<Project>); N]) -> Self {
        Self::from_iter(value)
    }
}

impl<Project: ProjectRead + Clone> From<Vec<(Iri<String>, Vec<Project>)>>
    for MemoryResolver<AcceptAll, Project>
{
    fn from(value: Vec<(Iri<String>, Vec<Project>)>) -> Self {
        Self::from_iter(value)
    }
}

pub trait IRIPredicate {
    fn accept_iri(&self, iri: &Iri<String>) -> bool;

    // TODO: be more efficient, don't clone
    fn accept_iri_raw(&self, iri: &str) -> bool {
        match Iri::parse(iri.to_string()) {
            Ok(iri) => self.accept_iri(&iri),
            Err(_) => false,
        }
    }
}

#[derive(Debug)]
pub struct AcceptAll {}

impl IRIPredicate for AcceptAll {
    fn accept_iri(&self, _iri: &Iri<String>) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct AcceptScheme<'a> {
    pub scheme: &'a Scheme,
}

impl IRIPredicate for AcceptScheme<'_> {
    fn accept_iri(&self, iri: &Iri<String>) -> bool {
        iri.scheme() == self.scheme
    }
}

impl<Predicate: IRIPredicate, ProjectStorage: ProjectRead + Clone> ResolveRead
    for MemoryResolver<Predicate, ProjectStorage>
{
    type Error = Infallible;

    type ProjectStorage = ProjectStorage;

    type ResolvedStorages = Vec<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        usage: &InterchangeProjectUsage,
        _base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let identifier = match usage {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint: _,
            } => {
                // TODO: should publisher/name identifiers be filtered?
                if !self.iri_predicate.accept_iri(resource) {
                    return Ok(ResolutionOutcome::Unresolvable(format!(
                        "invalid IRI `{resource}` for this memory resolver"
                    )));
                }
                Identifier::from_iri(resource)
            }
            _ => Identifier::from_interchange_usage(usage),
        };

        // TODO: be more efficient, avoid reparsing IRI. Maybe make `Identifier` contain `Iri<String>`?
        let iri: Iri<String> = identifier.into();
        Ok(match self.projects.get(&iri) {
            Some(xs) => ResolutionOutcome::Resolved(xs.iter().map(|x| Ok(x.clone())).collect()),
            None => ResolutionOutcome::NotFound(
                usage.to_owned(),
                String::from("memory resolver does not contain this project"),
            ),
        })
    }
}

// TODO: Add a memory resolver for certain ProjectStorage types?
