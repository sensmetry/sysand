// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    env::current_dir,
    path::{Path, PathBuf},
};

use crate::project::local_src::LocalSrcProject;

pub fn current_project() -> Result<Option<LocalSrcProject>, std::io::Error> {
    Ok(discover_project(current_dir()?))
}

// TODO: Improve the logic here, this is probably too simple
pub fn discover_project<P: AsRef<Path>>(working_directory: P) -> Option<LocalSrcProject> {
    let mut current: PathBuf = working_directory.as_ref().to_path_buf();

    log::debug!("Trying to discover project in {}", current.display());

    while !(current.join(".project.json").is_file() || current.join(".meta.json").is_file()) {
        match current.parent() {
            Some(parent) if parent == Path::new("") => {
                log::debug!("hit empty relative path, trying to canonicalise");
                match current.canonicalize() {
                    Ok(current_canonical) => match current_canonical.parent() {
                        Some(parent_canonical) => current = parent_canonical.to_path_buf(),
                        None => {
                            log::debug!(
                                "canonicalised path {} has no parent either",
                                current_canonical.display()
                            );
                            return None;
                        }
                    },
                    Err(_) => {
                        log::debug!("unable to canonicalise path {}", current.display());
                    }
                }
            }
            Some(parent) => {
                log::debug!("Checking for project in parent {}", current.display());
                current = parent.to_path_buf();
            }
            None => {
                return None;
            }
        }
    }

    Some(LocalSrcProject {
        project_path: current,
    })
}
