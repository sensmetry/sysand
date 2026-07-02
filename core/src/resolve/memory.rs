// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, convert::Infallible};

use fluent_uri::component::Scheme;

use crate::{
    model::InterchangeProjectUsage,
    project::{ProjectRead, utils::Identifier},
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead},
};

#[derive(Debug)]
pub struct MemoryResolver<Predicate, ProjectStorage: Clone> {
    pub iri_predicate: Predicate,
    pub projects: HashMap<Identifier, Vec<ProjectStorage>>,
}

impl<Project: ProjectRead + Clone> FromIterator<(Identifier, Vec<Project>)>
    for MemoryResolver<AcceptAll, Project>
{
    fn from_iter<T: IntoIterator<Item = (Identifier, Vec<Project>)>>(iter: T) -> Self {
        Self {
            iri_predicate: AcceptAll {},
            projects: HashMap::from_iter(iter),
        }
    }
}

impl<Project: ProjectRead + Clone, const N: usize> From<[(Identifier, Vec<Project>); N]>
    for MemoryResolver<AcceptAll, Project>
{
    fn from(value: [(Identifier, Vec<Project>); N]) -> Self {
        Self::from_iter(value)
    }
}

impl<Project: ProjectRead + Clone> From<Vec<(Identifier, Vec<Project>)>>
    for MemoryResolver<AcceptAll, Project>
{
    fn from(value: Vec<(Identifier, Vec<Project>)>) -> Self {
        Self::from_iter(value)
    }
}

pub trait IRIPredicate {
    fn accept(&self, usage: &ResolutionInfo) -> bool;
}

#[derive(Debug)]
pub struct AcceptAll {}

impl IRIPredicate for AcceptAll {
    fn accept(&self, _: &ResolutionInfo) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct AcceptScheme<'a> {
    pub scheme: &'a Scheme,
}

impl IRIPredicate for AcceptScheme<'_> {
    fn accept(&self, usage: &ResolutionInfo) -> bool {
        match usage.usage() {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint: _,
            } => resource.scheme() == self.scheme,
            InterchangeProjectUsage::Directory { .. } => false,
        }
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
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        if !self.iri_predicate.accept(resolve) {
            return Ok(ResolutionOutcome::UnsupportedUsageType {
                reason: String::from(
                    "this memory resolver is configured to not accept such a usage",
                ),
            });
        }

        let identifier = Identifier::from_interchange_usage(resolve.usage());
        Ok(match self.projects.get(&identifier) {
            Some(xs) => ResolutionOutcome::Resolved(xs.iter().map(|x| Ok(x.clone())).collect()),
            None => ResolutionOutcome::NotFound {
                reason: String::from("project is not present in this memory resolver"),
            },
        })
    }
}

// TODO: Add a memory resolver for certain ProjectStorage types?
