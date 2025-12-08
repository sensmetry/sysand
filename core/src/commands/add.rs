// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
use thiserror::Error;

use crate::{model::InterchangeProjectValidationError, project::ProjectMut};

#[derive(Error, Debug)]
pub enum AddError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
    #[error("missing project information: {0}")]
    MissingInfo(&'static str),
}

pub fn do_add<P: ProjectMut>(
    project: &mut P,
    iri: String,
    versions_constraint: Option<String>,
) -> Result<(), AddError<P::Error>> {
    let usage: crate::model::InterchangeProjectUsageRaw =
        crate::model::InterchangeProjectUsageRaw {
            resource: iri.clone(),
            version_constraint: versions_constraint.clone(),
        }
        .validate()?
        .into();

    let adding = "Adding";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{adding:>12}{header:#} usage: `{}` {}",
        &iri,
        versions_constraint
            .as_ref()
            .map(|vr| vr.to_string())
            .unwrap_or("".to_string()),
    );

    if let Some(info) = project.get_info().map_err(AddError::Project)?.as_mut() {
        // TODO: Would ideally try to merge version constraint
        //       rather than having multiple usages
        if !info.usage.contains(&usage) {
            info.usage.push(usage);
        }

        project.put_info(info, true).map_err(AddError::Project)?;

        Ok(())
    } else {
        Err(AddError::MissingInfo(
            "project is missing the interchange project information",
        ))
    }
}
