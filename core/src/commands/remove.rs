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
        "`{identifier}` is declared as a {kind} usage, not as a resource usage;\n\
        remove it with `sysand remove <publisher>/<name>`"
    )]
    UsageIsTyped {
        identifier: String,
        kind: &'static str,
    },
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

    if popped.is_empty() {
        // The same project may be declared as a typed usage, which has the
        // same `Identifier` but is not a resource usage. Saying "not found"
        // there would be false.
        if let UsageRef::Resource(iri) = usage
            && let Some(kind) = info.usage.iter().find_map(|usage| {
                (usage.is_typed()
                    && Identifier::from_unvalidated_usage(usage)
                        .is_some_and(|id| id.as_str() == iri.as_str()))
                .then(|| usage.kind_noun())
            })
        {
            return Err(RemoveError::UsageIsTyped {
                identifier: iri.to_string(),
                kind,
            });
        }
        Err(RemoveError::UsageNotFound(
            usage.to_string().into_boxed_str(),
        ))
    } else {
        project
            .put_info(&info, true)
            .map_err(RemoveError::Project)?;
        Ok(popped)
    }
}

#[cfg(test)]
#[path = "./remove_tests.rs"]
mod tests;
