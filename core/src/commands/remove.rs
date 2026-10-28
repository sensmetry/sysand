// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    model::{InterchangeProjectUsageRaw, InterchangeProjectValidationError, UsageRef},
    project::{ProjectFormat, ProjectMut, ProjectRead as _, utils::Identifier},
};

#[derive(Error, Debug)]
pub enum RemoveError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
    #[error("could not find usage for `{0}`")]
    UsageNotFound(Box<str>),
    /// No `Resource` usage of the IRI exists, but a typed usage of the *same
    /// project* does. Reserved as an error rather than reported as "not
    /// found", which would be untrue, so that removing by identifier can
    /// later be relaxed into removing the typed usage instead.
    #[error(
        "`{identifier}` is declared as {kind} usage, not as a resource usage;\n\
        remove it with `{remove_with}`"
    )]
    UsageIsTyped {
        identifier: String,
        kind: &'static str,
        /// The CLI command that removes it
        remove_with: String,
    },
    /// No usage has the given spelling, but one of the same project is
    /// spelled differently
    #[error("could not find usage for `{requested}`; did you mean `{existing}`?")]
    UsageSpelledDifferently { requested: String, existing: String },
    #[error("project is missing project information")]
    MissingInfo,
}

/// `remove` on a `model.toml` project.
///
/// This matches *more* than the json arm does for an IRI: there a
/// `Resource` usage is matched by its literal IRI, while a manifest
/// entry is matched by its normalized identity whatever kind it is. So
/// `sysand remove acme/lib`, or `sysand remove pkg:sysand/acme/lib`,
/// removes a `dir` entry here.
///
/// The usage has to name a project, so one identified by anything but a
/// `pkg:sysand` PURL or a `publisher/name` pair cannot be removed by
/// name -- which is the same reason it could not have been added.
fn remove_toml<P: ProjectMut>(
    project: &mut P,
    usage: &UsageRef<'_>,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    use crate::manifest::{common::normalized_identity, edit};
    use crate::model::{IndexName, IndexPublisher};

    let not_found = || RemoveError::UsageNotFound(usage.to_string().into_boxed_str());
    let (publisher, name) = match usage {
        UsageRef::Typed(publisher, name) => {
            (publisher.as_str().to_owned(), name.as_str().to_owned())
        }
        UsageRef::Resource(iri) => match crate::purl::parse_sysand_purl(iri.as_str()) {
            Ok(Some((publisher, name))) => (publisher.to_owned(), name.to_owned()),
            _ => return Err(not_found()),
        },
    };
    let publisher = IndexPublisher::parse(publisher).map_err(|_not_an_identity| not_found())?;
    let name = IndexName::parse(name).map_err(|_not_an_identity| not_found())?;
    let identity = normalized_identity(&publisher, &name);

    let mut document = project.manifest_document().map_err(|e| match e {
        crate::project::ManifestError::Project(e) => RemoveError::Project(e),
        crate::project::ManifestError::NoTomlManifest => RemoveError::MissingInfo,
    })?;

    edit::remove_usage(&mut document, &identity).map_err(|_not_declared| not_found())?;

    project
        .put_manifest_document(&document, true)
        .map_err(|e| match e {
            crate::project::ManifestWriteError::Project(e) => RemoveError::Project(e),
            crate::project::ManifestWriteError::Unsupported => RemoveError::MissingInfo,
        })?;

    // The json arm hands back what it removed; a manifest entry has no
    // `.project.json` shape, so nothing is handed back here.
    Ok(vec![])
}

/// Remove the usages matching `usage` according to [`UsageRef::matches`]
pub fn do_remove<P: ProjectMut>(
    project: &mut P,
    usage: UsageRef<'_>,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{removing:>12}{header:#} `{usage}` from usages");

    // `Both` falls through to the json arm, where `get_info` refuses it
    // as the ambiguity it is.
    if project.format().map_err(RemoveError::Project)? == ProjectFormat::Toml {
        return remove_toml(project, &usage);
    }

    let Some(mut info) = project.get_info().map_err(RemoveError::Project)? else {
        return Err(RemoveError::MissingInfo);
    };
    let popped = info.pop_usage(&usage);

    if !popped.is_empty() {
        project
            .put_info(&info, true)
            .map_err(RemoveError::Project)?;
        return Ok(popped);
    }

    match usage {
        // The same project may be declared as a typed usage, which has the
        // same `Identifier` but is not a resource usage. Saying "not found"
        // there would be false.
        UsageRef::Resource(iri) => {
            if let Some(usage) = info.usage.iter().find(|usage| {
                usage.is_typed()
                    && Identifier::from_unvalidated_usage(usage)
                        .is_some_and(|id| id.as_str() == iri.as_str())
            }) {
                return Err(RemoveError::UsageIsTyped {
                    identifier: iri.to_string(),
                    kind: usage.kind_with_article(),
                    remove_with: remove_command(usage),
                });
            }
        }
        // The same project may be declared as a typed usage, but with
        // different spelling
        // FIXME: a respelling is only found when it gets the same kind of
        // identifier as the declared spelling (see
        // `Identifier::from_project`). E.g. with `ACME Inc./Foo` declared
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

/// The CLI command that removes `usage`
fn remove_command(usage: &InterchangeProjectUsageRaw) -> String {
    /// Quote `arg` for a shell, if needed
    fn quoted(arg: &str) -> String {
        if arg.is_empty() || arg.contains(char::is_whitespace) {
            format!("\"{arg}\"")
        } else {
            arg.to_owned()
        }
    }
    match usage {
        InterchangeProjectUsageRaw::Resource {
            resource,
            version_constraint: _,
        } => {
            format!("sysand remove {}", quoted(resource))
        }
        InterchangeProjectUsageRaw::Directory {
            publisher,
            name,
            dir: _,
        }
        | InterchangeProjectUsageRaw::KparPath {
            publisher,
            name,
            kpar_path: _,
        }
        | InterchangeProjectUsageRaw::Index {
            publisher,
            name,
            version_constraint: _,
        } => format!("sysand remove {}", quoted(&format!("{publisher}/{name}"))),
    }
}

#[cfg(test)]
#[path = "./remove_tests.rs"]
mod tests;
