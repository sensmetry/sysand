// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use crate::{model::InterchangeProjectUsageRaw, project::ProjectMut};

#[derive(Error, Debug)]
pub enum RemoveError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error("could not find usage for {0}")]
    UsageNotFound(String),
    #[error("could not find project information for {0}")]
    MissingInfo(String),
}

pub fn do_remove<P: ProjectMut, S: AsRef<str>>(
    project: &mut P,
    iri: S,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{removing:>12}{header:#} {} from usages",
        iri.as_ref()
    );

    if let Some(mut info) = project.get_info().map_err(RemoveError::Project)? {
        let popped = info.pop_usage(&iri.as_ref().to_string());

        if popped.is_empty() {
            Err(RemoveError::UsageNotFound(iri.as_ref().to_string()))
        } else {
            project
                .put_info(&info, true)
                .map_err(RemoveError::Project)?;
            Ok(popped)
        }
    } else {
        Err(RemoveError::MissingInfo(iri.as_ref().to_string()))
    }
}
