// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    env::ReadEnvironment,
    manifest::usage::{MemberUsage, Usage},
    model::{
        IndexName, IndexPublisher, InterchangeProjectUsageRaw, InterchangeProjectValidationError,
    },
    project::{ProjectDefinition, ProjectFormat, ProjectMut, ProjectRead, utils::Identifier},
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
    /// the same [`Identifier`]. Multiple usages of one project are not
    /// allowed.
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
    /// The usage has no `model.toml` form; the message gives the reason.
    #[error(transparent)]
    NoManifestForm(crate::manifest::kpar_view::NoManifestForm),
    #[error(transparent)]
    Edit(crate::manifest::edit::EditError),
    #[error(transparent)]
    Manifest(#[from] crate::project::ManifestError<ProjectError>),
    #[error(transparent)]
    ManifestWrite(#[from] crate::project::ManifestWriteError<ProjectError>),
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
    #[error(
        "index usage `{usage}` cannot be used: version {version} installed in the local \
         environment declares itself `{spelling}`, which an index usage cannot spell"
    )]
    NotIndexSpelling {
        usage: String,
        version: String,
        spelling: String,
    },
    #[error(
        "index usage `{usage}` cannot be used: version {version} installed in the local \
         environment declares itself `{spelling}`, which is not the project the usage names"
    )]
    OtherProject {
        usage: String,
        version: String,
        spelling: String,
    },
}

/// The publisher and name an index usage of `publisher`/`name` has to spell,
/// read from the first version of the project installed in `env`: the
/// project's own spelling when `publisher`/`name` is normalized (see
/// [`IndexPublisher::is_normalized`]), and `publisher`/`name` itself
/// otherwise, once checked to be that spelling. `normalized` is whether
/// `publisher`/`name` is normalized.
///
/// Fails when no version is installed, or when the installed project
/// declares a publisher and name that do not normalize to `publisher`/`name`.
#[expect(clippy::type_complexity)]
pub fn spell_index_usage<Env: ReadEnvironment>(
    env: Option<&Env>,
    publisher: &IndexPublisher,
    name: &IndexName,
    normalized: bool,
) -> Result<
    (IndexPublisher, IndexName),
    IndexSpellingError<Env::ReadError, <Env::InterchangeProjectRead as ProjectRead>::Error>,
> {
    let usage = format!("{publisher}/{name}");
    let identifier = Identifier::from_index(publisher, name);

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
    let definition = env
        .get_project(identifier.as_str(), &version)
        .map_err(IndexSpellingError::Env)?
        .get_project()
        .map_err(IndexSpellingError::Project)?;
    let Some(definition) = definition else {
        return Err(IndexSpellingError::MissingInfo { usage, version });
    };
    let Some(found_publisher) = definition.publisher().map(ToOwned::to_owned) else {
        return Err(IndexSpellingError::NoPublisher { usage, version });
    };
    let found_name = definition.name().to_owned();
    if Identifier::from_pub_name(&found_publisher, &found_name) != identifier {
        return Err(IndexSpellingError::OtherProject {
            usage,
            version,
            spelling: format!("{found_publisher}/{found_name}"),
        });
    }
    if normalized || (found_publisher == publisher.as_str() && found_name == name.as_str()) {
        let spelling = format!("{found_publisher}/{found_name}");
        match (
            IndexPublisher::parse(found_publisher),
            IndexName::parse(found_name),
        ) {
            (Ok(publisher), Ok(name)) => Ok((publisher, name)),
            _ => Err(IndexSpellingError::NotIndexSpelling {
                usage,
                version,
                spelling,
            }),
        }
    } else {
        Err(IndexSpellingError::Misspelled {
            usage,
            version,
            spelling: format!("{found_publisher}/{found_name}"),
        })
    }
}

/// What [`merge_located_usage`] did with an existing usage.
enum Merge {
    /// The existing usage is the new one.
    Identical,
    /// The existing usage took the new location or the new publisher/name.
    Merged,
    /// The existing usage is of another project at another location.
    Unrelated,
}

/// The publisher of a usage that names a location: required for `dir`
/// and `kpar-path` usages, optional for a KPAR URL usage.
trait LocatedPublisher: PartialEq + Clone {
    /// The usage's publisher and name as the warnings quote them.
    fn quote(&self, name: &str) -> String;
}

impl LocatedPublisher for String {
    fn quote(&self, name: &str) -> String {
        format!("`{self}`/`{name}`")
    }
}

impl LocatedPublisher for Option<String> {
    fn quote(&self, name: &str) -> String {
        match self {
            Some(publisher) => publisher.quote(name),
            None => format!("`{name}`"),
        }
    }
}

/// Merge logic for usages that name a location and a publisher/name
/// (`Directory`, `KparPath`, `RemoteKpar`): if the existing usage matches
/// `new_publisher`/`new_name`, it takes `new_location`; if it matches
/// `new_location` instead, it takes the new publisher/name. `noun` names
/// the location in the warnings.
fn merge_located_usage<P: LocatedPublisher>(
    noun: &str,
    location: &mut String,
    publisher: &mut P,
    name: &mut String,
    new_location: &str,
    new_publisher: &P,
    new_name: &str,
) -> Merge {
    let same_project = publisher == new_publisher && name == new_name;
    let same_location = location == new_location;
    let existing = publisher.quote(name);
    if same_project && same_location {
        log::warn!(
            "ignoring usage {existing} at `{new_location}`,\n\
             {SP:>8} since it is already present"
        );
        Merge::Identical
    } else if same_project {
        log::warn!(
            "usage {existing} is already present;\n\
             {SP:>8} it will be updated to point to `{new_location}`"
        );
        new_location.clone_into(location);
        Merge::Merged
    } else if same_location {
        log::warn!(
            "existing usage {existing} already points to {noun}\n\
             {SP:>8} `{location}`; existing usage will be overwritten"
        );
        publisher.clone_from(new_publisher);
        new_name.clone_into(name);
        Merge::Merged
    } else {
        Merge::Unrelated
    }
}

/// The usage that `usages` declare of the project `identifier`, if any.
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
    /// [`IndexPublisher::is_normalized`]), and its constraint, if missing,
    /// have to be settled by the caller
    New {
        publisher: IndexPublisher,
        name: IndexName,
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
    publisher: IndexPublisher,
    name: IndexName,
    version_constraint: Option<semver::VersionReq>,
) -> Result<IndexUsageToAdd, AddError<E>> {
    let normalized = publisher.is_normalized() && name.is_normalized();
    let identifier = Identifier::from_index(&publisher, &name);
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
    // declared spelling is the one to use.
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

/// `add` on a `model.toml` project: append an `[[inline-usages]]` entry.
///
/// A usage already present is reported through the returned `bool`:
/// `false` means the usage was already there and nothing was written.
fn add_toml<P: ProjectMut>(
    project: &mut P,
    usage_raw: &InterchangeProjectUsageRaw,
) -> Result<bool, AddError<P::Error>> {
    use crate::commands::usage::{ConstraintChange, set_toml_index_constraint};
    use crate::manifest::edit;

    let usage =
        crate::manifest::kpar_view::usage_of_raw(usage_raw, crate::manifest::MODEL_TOML_FILE)
            .map_err(AddError::NoManifestForm)?;
    let identity = usage.identity();

    crate::project::edit_manifest_document(project, |_, document| {
        // Already declared: only a changed constraint needs writing.
        let declared = match &usage {
            Usage::Index {
                version_constraint,
                publisher: _,
                name: _,
            } => match set_toml_index_constraint(document, &identity, version_constraint)
                .map_err(AddError::Edit)?
            {
                ConstraintChange::Replaced { old: _, new: _ } => return Ok(true),
                ConstraintChange::Unchanged { constraint: _ } => true,
                ConstraintChange::NotFound => false,
            },
            _ => edit::usage_constraint(document, &identity).is_some(),
        };
        if declared {
            log::warn!("ignoring usage `{identity}`, since it is already present");
            return Ok(false);
        }

        edit::push_usage(document, &MemberUsage::Declared(usage)).map_err(AddError::Edit)?;
        Ok(true)
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
    let usage: InterchangeProjectUsageRaw = usage_raw.validate()?.into();

    let adding = "Adding";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{adding:>12}{header:#} usage: {usage_raw}");

    // `Both` falls through to the json arm, which refuses it as ambiguous.
    if project.format().map_err(AddError::Project)? == ProjectFormat::Toml {
        return add_toml(project, &usage);
    }

    if let Some(ProjectDefinition::Json { info, meta: _ }) =
        project.get_project().map_err(AddError::Project)?.as_mut()
    {
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
                    let InterchangeProjectUsageRaw::Directory {
                        dir,
                        publisher,
                        name,
                    } = u
                    else {
                        continue;
                    };
                    match merge_located_usage(
                        "path",
                        dir,
                        publisher,
                        name,
                        new_dir,
                        new_publisher,
                        new_name,
                    ) {
                        Merge::Identical => return Ok(false),
                        Merge::Merged => {
                            dont_add = true;
                            break;
                        }
                        Merge::Unrelated => {}
                    }
                }
            }
            InterchangeProjectUsageRaw::KparPath {
                kpar_path: new_kpar_path,
                publisher: new_publisher,
                name: new_name,
            } => {
                for u in &mut info.usage {
                    let InterchangeProjectUsageRaw::KparPath {
                        kpar_path,
                        publisher,
                        name,
                    } = u
                    else {
                        continue;
                    };
                    match merge_located_usage(
                        "file",
                        kpar_path,
                        publisher,
                        name,
                        new_kpar_path,
                        new_publisher,
                        new_name,
                    ) {
                        Merge::Identical => return Ok(false),
                        Merge::Merged => {
                            dont_add = true;
                            break;
                        }
                        Merge::Unrelated => {}
                    }
                }
            }
            InterchangeProjectUsageRaw::RemoteKpar {
                kpar_url: new_kpar_url,
                publisher: new_publisher,
                name: new_name,
            } => {
                for u in &mut info.usage {
                    let InterchangeProjectUsageRaw::RemoteKpar {
                        kpar_url,
                        publisher,
                        name,
                    } = u
                    else {
                        continue;
                    };
                    match merge_located_usage(
                        "KPAR URL",
                        kpar_url,
                        publisher,
                        name,
                        new_kpar_url,
                        new_publisher,
                        new_name,
                    ) {
                        Merge::Identical => return Ok(false),
                        Merge::Merged => {
                            dont_add = true;
                            break;
                        }
                        Merge::Unrelated => {}
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
