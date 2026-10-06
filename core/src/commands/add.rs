// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    env::ReadEnvironment,
    model::{
        InterchangeProjectUsageG, InterchangeProjectUsageRaw, InterchangeProjectValidationError,
    },
    project::{ProjectMut, ProjectRead, utils::Identifier},
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
    /// The project is already declared by a usage of a *different* kind with
    /// the same [`Identifier`]. Merging the two would mean choosing which
    /// source wins, so it is refused, and we don't allow multiple usages
    /// of the same project.
    ///
    /// [`Identifier`]: crate::project::utils::Identifier
    #[error(
        "`{identifier}` is already declared as {existing} usage;\n\
        remove it before adding it as {new} usage"
    )]
    DuplicateIdentifier {
        identifier: String,
        existing: &'static str,
        new: &'static str,
    },
    /// A typed usage of the same kind and project is already declared, but
    /// spelled differently. Only one of the spellings can match the
    /// project's own.
    #[error(
        "`{new}` is already declared as {kind} usage `{existing}`;\n\
        a typed usage must spell the publisher and name exactly as the project does"
    )]
    TypedUsageSpelledDifferently {
        /// With an article, e.g. "an index"
        kind: &'static str,
        existing: String,
        new: String,
    },
}

/// Why [`spell_index_usage`] could not settle the spelling of an index usage
#[derive(Error, Debug)]
pub enum IndexSpellingError<EnvError, ProjectError> {
    #[error(transparent)]
    Env(EnvError),
    #[error(transparent)]
    Project(ProjectError),
    #[error(
        "{}: it is not installed in the local environment",
        if *.normalized {
            format!("cannot find how the project `{usage}` spells its publisher and name")
        } else {
            format!("cannot check that `{usage}` is spelled as the project spells it")
        }
    )]
    NotInstalled { usage: String, normalized: bool },
    #[error(
        "version {version} of `{usage}` installed in the local environment has no project \
         information"
    )]
    MissingInfo { usage: String, version: String },
    #[error(
        "index usage `{usage}` cannot be used: version {version} installed in the local \
         environment declares no publisher"
    )]
    NoPublisher { usage: String, version: String },
    #[error(
        "index usage `{usage}` is rejected because its spelling does not match the project's: \
         version {version} installed in the local environment declares itself `{spelling}`;\n\
         spell the usage exactly as `{spelling}`"
    )]
    Misspelled {
        usage: String,
        version: String,
        spelling: String,
    },
}

/// The publisher and name an index usage of `publisher`/`name` has to spell,
/// read from a version of the project installed in `env` (the first it
/// lists: a project spells itself the same way in every version), without
/// touching the network: the project's own spelling when `publisher`/`name`
/// is normalized (see [`crate::purl::is_normalized_spelling`]), and `publisher`/`name`
/// itself otherwise, once it has been checked to be that spelling.
/// `normalized` is whether `publisher`/`name` is normalized, which the
/// caller has already found out.
///
/// Fails when no version is installed, since then there is nothing to check
/// against.
#[expect(clippy::type_complexity)]
pub fn spell_index_usage<Env: ReadEnvironment>(
    env: Option<&Env>,
    publisher: &str,
    name: &str,
    normalized: bool,
) -> Result<
    (String, String),
    IndexSpellingError<Env::ReadError, <Env::InterchangeProjectRead as ProjectRead>::Error>,
> {
    let usage = format!("{publisher}/{name}");
    let identifier = Identifier::from_pub_name(publisher, name);

    let version = match env {
        Some(env) => env
            .versions(identifier.as_str())
            .map_err(IndexSpellingError::Env)?
            .into_iter()
            .next()
            .transpose()
            .map_err(IndexSpellingError::Env)?,
        None => None,
    };
    let (Some(env), Some(version)) = (env, version) else {
        return Err(IndexSpellingError::NotInstalled { usage, normalized });
    };
    let info = env
        .get_project(identifier.as_str(), &version)
        .map_err(IndexSpellingError::Env)?
        .get_info()
        .map_err(IndexSpellingError::Project)?;
    let Some(info) = info else {
        return Err(IndexSpellingError::MissingInfo { usage, version });
    };
    let Some(found_publisher) = info.publisher else {
        return Err(IndexSpellingError::NoPublisher { usage, version });
    };
    let found_name = info.name;
    if normalized || (found_publisher == publisher && found_name == name) {
        Ok((found_publisher, found_name))
    } else {
        Err(IndexSpellingError::Misspelled {
            usage,
            version,
            spelling: format!("{found_publisher}/{found_name}"),
        })
    }
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

/// Ok(true) => project info changed: the usage was added, or an existing
/// usage of it was updated
/// Ok(false) => usage already present in project info as given
///
/// Accepts any usage kind. A usage of the same kind is updated: its path, or
/// its version constraint, which is replaced as `cargo add` does; a usage of
/// a *different* kind that identifies the same project is refused with
/// [`AddError::DuplicateIdentifier`].
pub fn do_add<P: ProjectMut>(
    project: &mut P,
    // TODO: take non-raw, CLI has it
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
                                if vc_new == vc_current {
                                    log::warn!(
                                        "ignoring usage `{new_resource}` with version constraint\n\
                                             {SP:>8} `{vc_new}`, since it is already present with identical version constraint",
                                    );
                                    return Ok(false);
                                } else {
                                    // Replaced, as `cargo add` does
                                    log::warn!(
                                        "usage `{new_resource}` is already present with version constraint\n\
                                             {SP:>8} `{vc_current}`, which is replaced by `{vc_new}`",
                                    );
                                    vc_new.clone_into(vc_current);
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
            InterchangeProjectUsageRaw::Index {
                publisher: new_publisher,
                name: new_name,
                version_constraint: new_vc,
            } => {
                for u in &mut info.usage {
                    let InterchangeProjectUsageRaw::Index {
                        publisher,
                        name,
                        version_constraint,
                    } = u
                    else {
                        continue;
                    };
                    if publisher != new_publisher || name != new_name {
                        continue;
                    }
                    if new_vc == version_constraint {
                        log::warn!(
                            "ignoring usage `{new_publisher}/{new_name}` with version constraint\n\
                             {SP:>8} `{new_vc}`, since it is already present with identical version constraint",
                        );
                        return Ok(false);
                    }
                    // Replaced, as `cargo add` does
                    log::warn!(
                        "usage `{new_publisher}/{new_name}` is already present with version\n\
                         {SP:>8} constraint `{version_constraint}`, which is replaced by `{new_vc}`",
                    );
                    new_vc.clone_into(version_constraint);
                    dont_add = true;
                    break;
                }
            }
        }
        if !dont_add {
            // Every same-kind usage spelled the same has been merged above, so
            // anything left sharing this usage's identity is either of the
            // same kind but spelled differently, or of a different kind: the
            // same project declared twice, from two sources.
            if let Some(identifier) = Identifier::from_unvalidated_usage(&usage)
                && let Some(existing) = info.usage.iter().find(|u| {
                    Identifier::from_unvalidated_usage(u).is_some_and(|id| id == identifier)
                })
            {
                if let (Some((publisher, name)), Some((new_publisher, new_name))) = (
                    existing.typed_publisher_name(),
                    usage.typed_publisher_name(),
                ) && std::mem::discriminant(existing) == std::mem::discriminant(&usage)
                {
                    return Err(AddError::TypedUsageSpelledDifferently {
                        kind: usage.kind_with_article(),
                        existing: format!("{publisher}/{name}"),
                        new: format!("{new_publisher}/{new_name}"),
                    });
                }
                return Err(AddError::DuplicateIdentifier {
                    identifier: identifier.into_string(),
                    existing: existing.kind_with_article(),
                    new: usage.kind_with_article(),
                });
            }
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
