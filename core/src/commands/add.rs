// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    env::ReadEnvironment,
    model::{
        InterchangeProjectUsageG, InterchangeProjectUsageRaw, InterchangeProjectValidationError,
        check_index_usage_spelling,
    },
    project::{ProjectMut, ProjectRead, utils::Identifier},
    purl::is_normalized_spelling,
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
/// is normalized (see [`is_normalized_spelling`]), and `publisher`/`name`
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

/// The usage that `usages` declare of the project `identifier`, if any.
/// `add` never declares a project twice.
fn declared<'a>(
    usages: &'a [InterchangeProjectUsageRaw],
    identifier: &Identifier,
) -> Option<&'a InterchangeProjectUsageRaw> {
    usages
        .iter()
        .find(|u| Identifier::from_unvalidated_usage(u).is_some_and(|id| id == *identifier))
}

/// Why a usage of the project `identifier`, of `kind` (with an article, see
/// [`InterchangeProjectUsageRaw::kind_with_article`]) and, if typed, spelled
/// `spelling`, cannot be added where `existing` already declares the project
/// and the two are not merged: spelled differently if they are of the same
/// kind, or the same project declared twice, from two sources, otherwise.
fn refusal<E>(
    identifier: Identifier,
    existing: &InterchangeProjectUsageRaw,
    kind: &'static str,
    spelling: Option<(&str, &str)>,
) -> AddError<E> {
    match (existing.typed_publisher_name(), spelling) {
        (Some((publisher, name)), Some((new_publisher, new_name)))
            if existing.kind_with_article() == kind =>
        {
            AddError::TypedUsageSpelledDifferently {
                kind,
                existing: format!("{publisher}/{name}"),
                new: format!("{new_publisher}/{new_name}"),
            }
        }
        _ => AddError::DuplicateIdentifier {
            identifier: identifier.into_string(),
            existing: existing.kind_with_article(),
            new: kind,
        },
    }
}

/// What adding an index usage amounts to, decided by [`index_usage_to_add`]
/// before anything is resolved
#[derive(Debug, PartialEq, Eq)]
pub enum IndexUsageToAdd {
    /// Declared with that spelling, and no constraint given: nothing to add
    AlreadyPresent,
    /// A complete usage, for [`do_add`]: spelled as declared, with the
    /// constraint given, which replaces the declared one
    Ready(InterchangeProjectUsageRaw),
    /// Not declared yet: its spelling, if normalized (see
    /// [`is_normalized_spelling`]), and its constraint, if missing, have to
    /// be settled by the caller
    New {
        publisher: String,
        name: String,
        version_constraint: Option<semver::VersionReq>,
        normalized: bool,
    },
}

/// What adding an index usage of `publisher`/`name`, with
/// `version_constraint` if given, to a project declaring `usages` amounts
/// to, before anything is resolved or looked up: a normalized spelling of
/// a project already declared takes the declared spelling, and the same
/// refusals as [`do_add`] apply.
pub fn index_usage_to_add<E>(
    usages: &[InterchangeProjectUsageRaw],
    publisher: String,
    name: String,
    version_constraint: Option<semver::VersionReq>,
) -> Result<IndexUsageToAdd, AddError<E>> {
    check_index_usage_spelling(&publisher, &name)?;
    let normalized = is_normalized_spelling(&publisher, &name);
    let identifier = Identifier::from_pub_name(&publisher, &name);
    let Some(existing) = declared(usages, &identifier) else {
        return Ok(IndexUsageToAdd::New {
            publisher,
            name,
            version_constraint,
            normalized,
        });
    };
    let spelling = (publisher.as_str(), name.as_str());
    let InterchangeProjectUsageRaw::Index {
        publisher: declared_publisher,
        name: declared_name,
        version_constraint: declared_constraint,
    } = existing
    else {
        return Err(refusal(identifier, existing, "an index", Some(spelling)));
    };
    // A normalized spelling names the project by its identifier only, so the
    // declared usage says how it is spelled
    if !normalized && (declared_publisher.as_str(), declared_name.as_str()) != spelling {
        return Err(refusal(identifier, existing, "an index", Some(spelling)));
    }
    Ok(match version_constraint {
        None => {
            log::warn!(
                "ignoring usage `{declared_publisher}/{declared_name}` without a version \
                 constraint,\n{SP:>8} since it is already present with version constraint\n\
                 {SP:>8} `{declared_constraint}`",
            );
            IndexUsageToAdd::AlreadyPresent
        }
        Some(version_constraint) => IndexUsageToAdd::Ready(InterchangeProjectUsageRaw::Index {
            publisher: declared_publisher.clone(),
            name: declared_name.clone(),
            version_constraint: version_constraint.to_string(),
        }),
    })
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
                && let Some(existing) = declared(&info.usage, &identifier)
            {
                return Err(refusal(
                    identifier,
                    existing,
                    usage.kind_with_article(),
                    usage.typed_publisher_name(),
                ));
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
