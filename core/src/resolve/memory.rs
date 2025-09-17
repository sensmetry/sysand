// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, convert::Infallible};

use fluent_uri::component::Scheme;

use crate::{
    project::ProjectRead,
    resolve::{ResolutionOutcome, ResolveRead},
};

#[derive(Debug)]
pub struct MemoryResolver<Predicate, ProjectStorage: Clone> {
    pub iri_predicate: Predicate,
    pub projects: HashMap<fluent_uri::Iri<String>, ProjectStorage>,
}

pub trait IRIPredicate {
    fn accept_iri(&self, iri: &fluent_uri::Iri<String>) -> bool;

    fn accept_iri_raw(&self, iri: &str) -> bool {
        match fluent_uri::Iri::parse(iri.to_string()) {
            Ok(iri) => self.accept_iri(&iri),
            Err(_) => false,
        }
    }
}

pub struct AcceptAll {}

impl IRIPredicate for AcceptAll {
    fn accept_iri(&self, _iri: &fluent_uri::Iri<String>) -> bool {
        true
    }
}

pub struct AcceptScheme<'a> {
    pub scheme: &'a Scheme,
}

impl IRIPredicate for AcceptScheme<'_> {
    fn accept_iri(&self, iri: &fluent_uri::Iri<String>) -> bool {
        iri.scheme() == self.scheme
    }
}

impl<Predicate: IRIPredicate, ProjectStorage: ProjectRead + Clone> ResolveRead
    for MemoryResolver<Predicate, ProjectStorage>
{
    type Error = Infallible;

    type ProjectStorage = ProjectStorage;

    type ResolvedStorages = std::iter::Once<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        if !self.iri_predicate.accept_iri(uri) {
            return Ok(ResolutionOutcome::UnsupportedIRIType(
                "Invalid IRI for this memory resolver".to_string(),
            ));
        }

        Ok(match self.projects.get(uri) {
            Some(x) => ResolutionOutcome::Resolved(std::iter::once(Ok(x.clone()))),
            None => ResolutionOutcome::Unresolvable(uri.to_string()),
        })
    }
}

// TODO: Add a memory resolver for certain ProjectStorage types?
