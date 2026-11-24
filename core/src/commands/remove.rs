// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::model::{InterchangeProjectUsageRaw, InterchangeProjectValidationError};
#[cfg(feature = "filesystem")]
use crate::{
    context::ManifestEdit,
    manifest::project::ResolvedProject,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, UsageRef},
    project::{
        local_src::{LocalSrcError, LocalSrcProject},
        utils::Identifier,
    },
};

#[cfg(test)]
use crate::{commands::edit_testing::edit_in_memory, project::memory::InMemoryProject};

#[derive(Error, Debug)]
pub enum RemoveError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
    #[error("could not find usage for `{0}`")]
    UsageNotFound(Box<str>),
    /// No `Resource` usage of the IRI exists, but a typed usage of the *same
    /// project* does.
    #[error("`{identifier}` is declared as {kind} usage, not as a resource usage")]
    UsageIsTyped {
        identifier: String,
        kind: &'static str,
        /// The declared usage
        usage: Box<InterchangeProjectUsageRaw>,
    },
    /// No usage has the given spelling, but one of the same project is
    /// spelled differently
    #[error("could not find usage for `{requested}`; did you mean `{existing}`?")]
    UsageSpelledDifferently { requested: String, existing: String },
    #[error("project is missing project information")]
    MissingInfo,
}

impl<ProjectError> From<crate::project::ManifestError<ProjectError>> for RemoveError<ProjectError> {
    fn from(error: crate::project::ManifestError<ProjectError>) -> Self {
        match error {
            crate::project::ManifestError::Project(e) => Self::Project(e),
            crate::project::ManifestError::NoTomlManifest => Self::MissingInfo,
        }
    }
}

impl<ProjectError> From<crate::project::ManifestWriteError<ProjectError>>
    for RemoveError<ProjectError>
{
    fn from(error: crate::project::ManifestWriteError<ProjectError>) -> Self {
        match error {
            crate::project::ManifestWriteError::Project(e) => Self::Project(e),
            crate::project::ManifestWriteError::Unsupported => Self::MissingInfo,
        }
    }
}

#[cfg(feature = "filesystem")]
pub(crate) fn remove_toml<E>(
    document: &mut toml_edit::DocumentMut,
    usage: &UsageRef<'_>,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<E>> {
    use crate::manifest::{
        common::{NormalizedIdentity, normalized_identity},
        edit,
    };
    use crate::model::{IndexName, IndexPublisher};

    let not_found = || RemoveError::UsageNotFound(usage.to_string().into_boxed_str());
    let identity = match usage {
        UsageRef::Typed(publisher, name) => {
            let publisher = IndexPublisher::parse(publisher.as_str())
                .map_err(|_not_an_identity| not_found())?;
            let name = IndexName::parse(name.as_str()).map_err(|_not_an_identity| not_found())?;
            normalized_identity(&publisher, &name)
        }
        UsageRef::Resource(iri) => match NormalizedIdentity::from_purl(iri.as_str()) {
            Ok(Some(identity)) => identity,
            Ok(None) | Err(_) => return Err(not_found()),
        },
    };

    edit::remove_usage(document, &identity).map_err(|_not_declared| not_found())?;

    // A manifest entry has no `.project.json` shape to hand back.
    Ok(vec![])
}

/// Removing the usages that match `usage` from the current project, as
/// [`UsageRef::matches`] decides, and reporting the usages removed. A
/// `model.toml` entry has no `.project.json` shape to hand back, so none are
/// reported for it.
#[cfg(feature = "filesystem")]
#[derive(Debug)]
pub struct RemoveUsage<'a> {
    usage: UsageRef<'a>,
}

#[cfg(feature = "filesystem")]
impl<'a> RemoveUsage<'a> {
    pub fn new(usage: UsageRef<'a>) -> Self {
        let removing = "Removing";
        let header = crate::style::get_style_config().header;
        log::info!("{header}{removing:>12}{header:#} `{usage}` from usages");
        Self { usage }
    }
}

#[cfg(feature = "filesystem")]
impl ManifestEdit for RemoveUsage<'_> {
    type Output = Vec<InterchangeProjectUsageRaw>;
    type Error = RemoveError<LocalSrcError>;

    fn toml(
        self,
        document: &mut toml_edit::DocumentMut,
        _project: &ResolvedProject,
    ) -> Result<Self::Output, Self::Error> {
        remove_toml(document, &self.usage)
    }

    fn json(
        self,
        _project: &LocalSrcProject,
        info: &mut InterchangeProjectInfoRaw,
        _meta: &mut InterchangeProjectMetadataRaw,
    ) -> Result<Self::Output, Self::Error> {
        remove_json(info, &self.usage)
    }
}

/// `remove` on a json project.
#[cfg(feature = "filesystem")]
pub(crate) fn remove_json<E>(
    info: &mut InterchangeProjectInfoRaw,
    usage: &UsageRef<'_>,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<E>> {
    let popped = info.pop_usage(usage);

    if !popped.is_empty() {
        return Ok(popped);
    }

    match *usage {
        // The same project may be declared as a typed usage, which has the
        // same `Identifier` but is not a resource usage.
        UsageRef::Resource(iri) => {
            if let Some(usage) = info.usage.iter().find(|usage| {
                usage.is_typed()
                    && Identifier::from_unvalidated_usage(usage)
                        .is_some_and(|id| id.as_str() == iri.as_str())
            }) {
                return Err(RemoveError::UsageIsTyped {
                    identifier: iri.to_string(),
                    kind: usage.kind_with_article(),
                    usage: Box::new(usage.clone()),
                });
            }
        }
        // The same project may be declared as a typed usage with a
        // different spelling.
        // FIXME: a respelling is only found when it gets the same kind of
        // identifier as the declared spelling. E.g. with `ACME Inc./Foo` declared
        // (`urn:sysand:acme-inc/foo`), `Acme Inc./FOO` is reported as spelled
        // differently, but `ACME Inc/Foo` (`pkg:sysand/acme-inc/foo`) only as
        // not found
        UsageRef::Typed(publisher, name) => {
            let identifier = Identifier::from_project(publisher, name);
            if let Some(
                InterchangeProjectUsageRaw::Directory {
                    publisher: p,
                    name: n,
                    dir: _,
                }
                | InterchangeProjectUsageRaw::KparPath {
                    publisher: p,
                    name: n,
                    kpar_path: _,
                }
                | InterchangeProjectUsageRaw::Index {
                    publisher: p,
                    name: n,
                    version_constraint: _,
                },
            ) = info.usage.iter().find(|usage| {
                Identifier::from_unvalidated_usage(usage).is_some_and(|id| id == identifier)
            }) {
                return Err(RemoveError::UsageSpelledDifferently {
                    requested: format!("{publisher}/{name}"),
                    existing: format!("{p}/{n}"),
                });
            }
        }
    }
    Err(RemoveError::UsageNotFound(
        usage.to_string().into_boxed_str(),
    ))
}

/// [`RemoveUsage`] on an in-memory project.
#[cfg(test)]
pub(crate) fn remove_in_memory(
    project: &mut InMemoryProject,
    usage: UsageRef<'_>,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<std::convert::Infallible>> {
    edit_in_memory(
        project,
        |document, _| remove_toml(document, &usage),
        |info, _| remove_json(info, &usage),
    )
}

#[cfg(test)]
#[path = "./remove_tests.rs"]
mod tests;
