// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use crate::{model::InterchangeProjectUsageRaw, project::ProjectMut};

#[derive(Error, Debug)]
pub enum RemoveError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error("could not find usage for `{0}`")]
    UsageNotFound(Box<str>),
    #[error("could not find usage with publisher `{0}`, name `{1}`")]
    ExperimentalUsageNotFound(Box<str>, Box<str>),
    #[error("current project info was removed")]
    MissingInfo,
}

pub fn do_remove<P: ProjectMut, S: AsRef<str>>(
    current_project: &mut P,
    iri: S,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{removing:>12}{header:#} `{}` from usages",
        iri.as_ref()
    );

    if let Some(mut info) = current_project.get_info().map_err(RemoveError::Project)? {
        let popped = info.pop_usage(&iri.as_ref().to_string());

        if popped.is_empty() {
            Err(RemoveError::UsageNotFound(iri.as_ref().into()))
        } else {
            current_project
                .put_info(&info, true)
                .map_err(RemoveError::Project)?;
            Ok(popped)
        }
    } else {
        Err(RemoveError::MissingInfo)
    }
}

pub fn do_remove_experimental<P: ProjectMut>(
    current_project: &mut P,
    publisher: impl AsRef<str>,
    name: impl AsRef<str>,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let publisher = publisher.as_ref();
    let name = name.as_ref();
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{removing:>12}{header:#} project `{publisher}`/`{name}` from usages");

    if let Some(mut info) = current_project.get_info().map_err(RemoveError::Project)? {
        let popped = info.pop_usage_experimental(publisher, name);

        if popped.is_empty() {
            Err(RemoveError::ExperimentalUsageNotFound(
                publisher.into(),
                name.into(),
            ))
        } else {
            current_project
                .put_info(&info, true)
                .map_err(RemoveError::Project)?;
            Ok(popped)
        }
    } else {
        Err(RemoveError::MissingInfo)
    }
}
