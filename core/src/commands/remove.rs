// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use crate::{model::InterchangeProjectUsageRaw, project::ProjectMut};

#[derive(Error, Debug)]
pub enum RemoveError<ProjectError> {
    #[error("{0}")]
    ProjectError(ProjectError),
}

pub fn do_remove<P: ProjectMut, S: AsRef<str>>(
    project: &mut P,
    iri: S,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    if let Some(mut info) = project.get_info().map_err(RemoveError::ProjectError)? {
        let popped = info.pop_usage(&iri.as_ref().to_string());

        if !popped.is_empty() {
            project
                .put_info(&info, true)
                .map_err(RemoveError::ProjectError)?;
        }

        Ok(popped)
    } else {
        Ok(vec![])
    }
}
