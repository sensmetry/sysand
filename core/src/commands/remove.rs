// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    add::expand_sysand_purl_shorthand,
    model::{InterchangeProjectUsageRaw, InterchangeProjectValidationError},
    project::ProjectMut,
};

#[derive(Error, Debug)]
pub enum RemoveError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
    #[error("could not find usage for `{0}`")]
    UsageNotFound(Box<str>),
    #[error("could not find project information for `{0}`")]
    MissingInfo(Box<str>),
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
                InterchangeProjectValidationError::MalformedSysandPurl {
                    iri: resource,
                    source,
                },
            ));
        }
    };
    do_remove(project, iri)
}

pub fn do_remove<P: ProjectMut>(
    project: &mut P,
    iri: String,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{removing:>12}{header:#} `{}` from usages", iri);

    if let Some(mut info) = project.get_info().map_err(RemoveError::Project)? {
        let popped = info.pop_usage(&iri);

        if popped.is_empty() {
            Err(RemoveError::UsageNotFound(iri.into_boxed_str()))
        } else {
            project
                .put_info(&info, true)
                .map_err(RemoveError::Project)?;
            Ok(popped)
        }
    } else {
        Err(RemoveError::MissingInfo(iri.into_boxed_str()))
    }
}

#[cfg(test)]
#[path = "./remove_tests.rs"]
mod tests;
