// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
use thiserror::Error;

use crate::{
    model::{
        InterchangeProjectUsageG, InterchangeProjectUsageRaw, InterchangeProjectValidationError,
    },
    project::ProjectMut,
};

const SP: char = ' ';

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
    usage_raw: &InterchangeProjectUsageRaw,
) -> Result<(), AddError<P::Error>> {
    let usage: InterchangeProjectUsageG<String, String> = usage_raw.validate()?.into();

    let adding = "Adding";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{adding:>12}{header:#} usage: `{}` {}",
        &usage_raw.resource,
        usage_raw
            .version_constraint
            .as_ref()
            .map(|vr| vr.to_string())
            .unwrap_or("".to_string()),
    );

    if let Some(info) = project.get_info().map_err(AddError::Project)?.as_mut() {
        if let Some(u) = info.usage.iter_mut().find(|u| u.resource == usage.resource) {
            match (usage.version_constraint, &mut u.version_constraint) {
                (None, None) => log::warn!(
                    "ignoring usage `{}`,\n\
                    {SP:>8} since it is already present",
                    usage.resource,
                ),
                (None, Some(vc)) => log::warn!(
                    "ignoring usage `{}`\n\
                    {SP:>8} without a version constraint, since it is already present with\n\
                    {SP:>8} version constraint `{}`",
                    usage.resource,
                    vc
                ),
                (Some(vc), vc_current @ None) => {
                    log::warn!(
                        "usage `{}` is already present,\n\
                        {SP:>8} but without a version constraint; version constraint\n\
                        {SP:>8} `{}` will be added to it",
                        usage.resource,
                        vc
                    );
                    *vc_current = Some(vc);
                }
                (Some(vc_new), Some(vc_current)) => {
                    // TODO: more intelligent merging of constraints
                    if &vc_new == vc_current {
                        log::warn!(
                            "ignoring usage `{}` with version constraint\n\
                            {SP:>8} `{}`, since it is already present with identical version constraint",
                            usage.resource,
                            vc_new
                        )
                    } else {
                        log::warn!(
                            "usage `{}` is already present, but with version\n\
                            {SP:>8} constraint `{}`; new version constraint\n\
                            {SP:>8} `{}` will be added to the existing ones; this may\n\
                            {SP:>8} result in failed version resolution or conflicting symbol errors",
                            u.resource,
                            vc_current,
                            vc_new
                        );
                        vc_current.push_str(", ");
                        vc_current.push_str(&vc_new);
                    }
                }
            }
        } else {
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
