// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    add::expand_sysand_purl_shorthand,
    model::{InterchangeProjectUsageRaw, InterchangeProjectValidationError},
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
    #[error("could not find usage for `{publisher}/{name}`")]
    ExpUsageNotFound { publisher: String, name: String },
    /// No `Resource` usage of the IRI exists, but a typed usage of the *same
    /// project* does. Reserved as an error rather than reported as "not
    /// found", which would be untrue, so that removing by identifier can
    /// later be relaxed into removing the typed usage instead.
    #[error(
        "`{identifier}` is declared as a {kind} usage, not as a resource usage;\n\
        remove it with `sysand experimental remove <publisher> <name>`"
    )]
    UsageIsTyped {
        identifier: String,
        kind: &'static str,
    },
    #[error("project is missing project information")]
    MissingInfo,
}

/// Like `do_remove`, but try to guess how `resource` should be interpreted.
/// Currently it can be either an IRI or `publisher/name` PURL shorthand
pub fn do_remove_guess<P: ProjectMut>(
    project: &mut P,
    resource: String,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let iri = match expand_sysand_purl_shorthand(&resource) {
        Ok(Some(purl)) => purl,
        Ok(None) => resource,
        Err(source) => {
            return Err(RemoveError::Validation(
                InterchangeProjectValidationError::MalformedUsageSysandPurl {
                    iri: resource,
                    source,
                },
            ));
        }
    };
    do_remove(project, iri)
}

/// Remove the `Resource` usage naming `iri`.
///
/// Typed usages are not removed, even when they identify the same project:
/// that case is refused with [`RemoveError::UsageIsTyped`] rather than
/// reported as missing.
pub fn do_remove<P: ProjectMut>(
    project: &mut P,
    iri: String,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{removing:>12}{header:#} `{iri}` from usages");

    if let Some(mut info) = project.get_info().map_err(RemoveError::Project)? {
        let popped = info.pop_usage(&iri);

        if popped.is_empty() {
            // The same project may be declared as a typed usage, which has the
            // same `Identifier` but is not a resource usage. Saying "not found"
            // there would be false.
            if let Some(kind) = info.usage.iter().find_map(|usage| {
                (usage.is_typed()
                    && Identifier::from_unvalidated_usage(usage)
                        .is_some_and(|id| id.as_str() == iri))
                .then(|| usage.kind_noun())
            }) {
                return Err(RemoveError::UsageIsTyped {
                    identifier: iri,
                    kind,
                });
            }
            Err(RemoveError::UsageNotFound(iri.into_boxed_str()))
        } else {
            project
                .put_info(&info, true)
                .map_err(RemoveError::Project)?;
            Ok(popped)
        }
    } else {
        Err(RemoveError::MissingInfo)
    }
}

pub fn exp_do_remove<P: ProjectMut>(
    project: &mut P,
    publisher: &str,
    name: &str,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{removing:>12}{header:#} `{publisher}/{name}` from usages");

    if let Some(mut info) = project.get_info().map_err(RemoveError::Project)? {
        let popped = info.exp_pop_usage(publisher, name);

        if popped.is_empty() {
            Err(RemoveError::ExpUsageNotFound {
                publisher: publisher.to_owned(),
                name: name.to_owned(),
            })
        } else {
            project
                .put_info(&info, true)
                .map_err(RemoveError::Project)?;
            Ok(popped)
        }
    } else {
        Err(RemoveError::MissingInfo)
    }
}

#[cfg(test)]
#[path = "./remove_tests.rs"]
mod tests;
