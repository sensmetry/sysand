// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
use thiserror::Error;

use crate::{
    model::{
        InterchangeProjectUsageG, InterchangeProjectUsageRaw, InterchangeProjectValidationError,
    },
    project::ProjectMut,
    purl::{PKG_SYSAND_PREFIX, SysandPurlError, parse_sysand_purl},
    utils::SP,
};

#[derive(Error, Debug)]
pub enum AddError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
    #[error("missing project information: {0}")]
    MissingInfo(&'static str),
}

/// If `resource` is of shape `publisher/name`, and both satisfy Sysand PURL
/// rules, return `Ok(Some(pkg:sysand/publisher/name))`. Otherwise, if it's of shape
/// `string1/string2` and does not contain `:`, return error. If none of these,
/// return `Ok(None)`, which indicates that it's likely something else
pub fn expand_sysand_purl_shorthand(resource: &str) -> Result<Option<String>, SysandPurlError> {
    let mut parts = resource.split('/');
    let publisher = parts.next();
    let name = parts.next();
    let has_exactly_two_segments = publisher.is_some() && name.is_some() && parts.next().is_none();

    // IRI always starts with `scheme:`, so differentiate from it by absence of `:`
    if !resource.contains(':') && has_exactly_two_segments {
        let purl = format!("{PKG_SYSAND_PREFIX}{resource}");
        match parse_sysand_purl(&purl) {
            Ok(Some(_)) => Ok(Some(purl)),
            Err(SysandPurlError::WrongShape { .. }) | Ok(None) => unreachable!(),
            Err(source) => Err(source),
        }
    } else {
        Ok(None)
    }
}

/// Like `do_add`, but try to guess how `resource` should be interpreted.
/// Currently it can be either an IRI or `publisher/name` PURL shorthand
pub fn do_add_guess<P: ProjectMut>(
    project: &mut P,
    resource: String,
    version_constraint: Option<String>,
) -> Result<bool, AddError<P::Error>> {
    let usage_raw = InterchangeProjectUsageRaw::Resource {
        resource: match expand_sysand_purl_shorthand(&resource) {
            Ok(Some(purl)) => purl,
            Ok(None) => resource,
            Err(source) => {
                return Err(AddError::Validation(
                    InterchangeProjectValidationError::MalformedUsageSysandPurl {
                        iri: resource,
                        source,
                    },
                ));
            }
        },
        version_constraint,
    };
    do_add(project, &usage_raw)
}

/// Common merge logic for path-like usages (`Directory`, `KparPath`): if an
/// existing usage matches `new_publisher`/`new_name`, update its path; if it
/// matches `new_path` instead, overwrite its publisher/name. Returns `true`
/// if a match was found and merged (and thus the new usage should not be
/// added separately), or `false` if there was no match.
fn try_merge_path_usage(
    noun: &str,
    path: &mut String,
    publisher: &mut String,
    name: &mut String,
    new_path: &str,
    new_publisher: &str,
    new_name: &str,
) -> bool {
    if publisher == new_publisher && name == new_name {
        log::warn!(
            "usage `{publisher}`/`{name}` is already present;\n\
            {SP:>8} it will be updated to point to `{new_path}`"
        );
        *path = new_path.to_owned();
        true
    } else if path == new_path {
        log::warn!(
            "existing usage `{publisher}`/`{name}` already points to {noun}
            {SP:>8} `{path}`; existing usage will be overwritten"
        );
        *publisher = new_publisher.to_owned();
        *name = new_name.to_owned();
        true
    } else {
        false
    }
}

/// Ok(true) => usage added to project info
/// Ok(false) => usage already present in project info
pub fn do_add<P: ProjectMut>(
    project: &mut P,
    usage_raw: &InterchangeProjectUsageRaw,
) -> Result<bool, AddError<P::Error>> {
    let usage: InterchangeProjectUsageG<String, String, String> = usage_raw.validate()?.into();

    let adding = "Adding";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{adding:>12}{header:#} usage: {usage_raw}");

    if let Some(info) = project.get_info().map_err(AddError::Project)?.as_mut() {
        let mut dont_add = false;
        match &usage {
            InterchangeProjectUsageRaw::Resource {
                resource: new_resource,
                version_constraint: new_vc,
            } => {
                for u in &mut info.usage {
                    if let InterchangeProjectUsageRaw::Resource {
                        resource,
                        version_constraint,
                    } = u
                        && resource == new_resource
                    {
                        match (&new_vc, version_constraint) {
                            (None, None) => {
                                log::warn!(
                                    "ignoring usage `{new_resource}`,\n\
                                         {SP:>8} since it is already present"
                                );
                                return Ok(false);
                            }
                            (None, Some(vc)) => {
                                log::warn!(
                                    "ignoring usage `{new_resource}`\n\
                                         {SP:>8} without a version constraint, since it is already present with\n\
                                         {SP:>8} version constraint `{vc}`",
                                );
                                return Ok(false);
                            }
                            (Some(vc), vc_current @ None) => {
                                log::warn!(
                                    "usage `{new_resource}` is already present,\n\
                                         {SP:>8} but without a version constraint; version constraint\n\
                                         {SP:>8} `{vc}` will be added to it",
                                );
                                *vc_current = Some(vc.to_owned());
                                dont_add = true;
                            }
                            (Some(vc_new), Some(vc_current)) => {
                                // TODO: more intelligent merging of constraints
                                if vc_new == vc_current {
                                    log::warn!(
                                        "ignoring usage `{new_resource}` with version constraint\n\
                                             {SP:>8} `{vc_new}`, since it is already present with identical version constraint",
                                    );
                                    return Ok(false);
                                } else {
                                    log::warn!(
                                        "usage `{new_resource}` is already present, but with version\n\
                                             {SP:>8} constraint `{vc_current}`; new version constraint\n\
                                             {SP:>8} `{vc_new}` will be added to the existing ones; this may\n\
                                             {SP:>8} result in failed version resolution or conflicting symbol errors",
                                    );
                                    vc_current.push_str(", ");
                                    vc_current.push_str(vc_new);
                                    dont_add = true;
                                }
                            }
                        }
                        break;
                    }
                }
            }
            InterchangeProjectUsageRaw::Directory {
                dir: new_dir,
                publisher: new_publisher,
                name: new_name,
            } => {
                // It might be desirable to merge different path-like usages
                // (`file:`, dir, kpar_path) or alternatively error out if
                // a path-like usage of that path already exists, but it is not
                // currently done.
                for u in &mut info.usage {
                    if let InterchangeProjectUsageRaw::Directory {
                        dir,
                        publisher,
                        name,
                    } = u
                        && try_merge_path_usage(
                            "path",
                            dir,
                            publisher,
                            name,
                            new_dir,
                            new_publisher,
                            new_name,
                        )
                    {
                        dont_add = true;
                        break;
                    }
                }
            }
            InterchangeProjectUsageRaw::KparPath {
                kpar_path: new_kpar_path,
                publisher: new_publisher,
                name: new_name,
            } => {
                for u in &mut info.usage {
                    if let InterchangeProjectUsageRaw::KparPath {
                        kpar_path,
                        publisher,
                        name,
                    } = u
                        && try_merge_path_usage(
                            "file",
                            kpar_path,
                            publisher,
                            name,
                            new_kpar_path,
                            new_publisher,
                            new_name,
                        )
                    {
                        dont_add = true;
                        break;
                    }
                }
            }
        }
        if !dont_add {
            info.usage.push(usage);
        }
        project.put_info(info, true).map_err(AddError::Project)?;
        Ok(true)
    } else {
        Err(AddError::MissingInfo(
            "project is missing the interchange project information",
        ))
    }
}

#[cfg(test)]
#[path = "./add_tests.rs"]
mod tests;
