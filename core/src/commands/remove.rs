// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    model::{InterchangeProjectUsageRaw, InterchangeProjectValidationError, UsageRef},
    project::{ProjectMut, utils::Identifier},
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

/// Remove the usages matching `usage` according to [`UsageRef::matches`]
pub fn do_remove<P: ProjectMut>(
    project: &mut P,
    usage: UsageRef<'_>,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{removing:>12}{header:#} `{usage}` from usages");

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
            let identifier = Identifier::from_pub_name(publisher, name);
            if let Some(
                InterchangeProjectUsageRaw::Directory {
                    publisher: p,
                    name: n,
                    ..
                }
                | InterchangeProjectUsageRaw::KparPath {
                    publisher: p,
                    name: n,
                    ..
                }
                | InterchangeProjectUsageRaw::Index {
                    publisher: p,
                    name: n,
                    ..
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
        InterchangeProjectUsageRaw::Resource { resource, .. } => {
            format!("sysand remove {}", quoted(resource))
        }
        InterchangeProjectUsageRaw::Directory {
            publisher, name, ..
        }
        | InterchangeProjectUsageRaw::KparPath {
            publisher, name, ..
        }
        | InterchangeProjectUsageRaw::Index {
            publisher, name, ..
        } => format!("sysand remove {}", quoted(&format!("{publisher}/{name}"))),
    }
}

#[cfg(test)]
#[path = "./remove_tests.rs"]
mod tests;
