// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `sysand migrate`: a json project's two documents become a
//! `model.toml`.
//!
//! This module is the conversion alone -- no filesystem, no network, no
//! working-tree check. What it cannot decide from the two documents is
//! reported as an error rather than guessed, except for three `resource`
//! usage shapes that need the target read: those go to the caller's
//! [`TargetLookup`].
//!
//! model-toml.md "JSON to TOML" is the field table; "Mixed manifests"
//! is why this is the one command that reads both documents.

use indexmap::IndexSet;
use semver::{Version, VersionReq};
use typed_path::Utf8UnixPathBuf;

use crate::{
    manifest::{
        ModelToml,
        common::{Maintainer, Sources},
        diagnostics::{Diagnostic, ReadWarning, Report, Validated},
        inherit::MetaProperty,
        project::{ProjectManifest, ProjectTable, Urls},
        usage::{MemberUsage, Usage},
    },
    model::{
        IndexName, IndexPublisher, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsageRaw, Metamodel,
    },
    purl::parse_sysand_purl,
    symbols::Language,
    utils::{RelativePathKind, normalize_relative, parse_relative_unix_path},
};

/// What the caller supplies that the documents do not.
#[derive(Clone, Debug, Default)]
pub struct MigrateOptions {
    /// `--publisher`. Used when the json project declares none;
    /// model-toml.md gives the file's own value "the same treatment as
    /// `name`", so a declared publisher is migrated rather than
    /// overridden.
    pub publisher: Option<String>,
}

/// Convert a json project's documents into a `model.toml` manifest.
///
/// All-or-nothing: every problem is collected, so a project with three
/// mistakes is reported once rather than three times.
pub fn migrate_project(
    info: &InterchangeProjectInfoRaw,
    meta: &InterchangeProjectMetadataRaw,
    options: &MigrateOptions,
) -> Result<Validated<ModelToml>, Report> {
    let mut report = Report::new();

    let name = parse_or_report(IndexName::parse(info.name.clone()), "name", &mut report);
    let publisher = migrate_publisher(info, options, &mut report);
    let version = match Version::parse(&info.version) {
        Ok(version) => Some(version),
        Err(error) => {
            report.push(Diagnostic::error(format!(
                "`version` is `{}`, which is not a semantic version: {error}",
                info.version
            )));
            None
        }
    };

    let sources = migrate_sources(meta, &mut report);
    let usages = migrate_usages(info, &mut report);
    let license = info
        .license
        .as_ref()
        .and_then(|raw| match spdx::Expression::parse(raw) {
            Ok(expression) => Some(expression),
            Err(error) => {
                report.push(Diagnostic::error(format!(
                    "`license` is `{raw}`, which is not a valid SPDX expression: {error}"
                )));
                None
            }
        });
    let maintainers: Vec<Maintainer> = info
        .maintainer
        .iter()
        .filter_map(|raw| match Maintainer::parse(raw) {
            Ok(maintainer) => Some(maintainer),
            Err(error) => {
                report.push(Diagnostic::error(format!(
                    "`maintainer` entry `{raw}` is not a valid maintainer: {error}"
                )));
                None
            }
        })
        .collect();
    let website = info
        .website
        .as_ref()
        .and_then(|raw| match fluent_uri::Iri::parse(raw.clone()) {
            Ok(iri) => Some(iri),
            Err((error, _)) => {
                report.push(Diagnostic::error(format!(
                    "`website` is `{raw}`, which is not a valid IRI: {error}"
                )));
                None
            }
        });
    let metamodel = migrate_metamodel(meta, &mut report);

    warn_discarded(meta, &mut report);

    // Every value is built before this point, so the report holds every
    // problem rather than the first.
    let (Some(name), Some(publisher), Some(version)) = (name, publisher, version) else {
        return Err(report);
    };

    let mut project = ProjectTable::new(publisher, name, version);
    project.sources = Some(sources.unwrap_or_default());
    project.description = info.description.clone();
    project.license = license.map(MetaProperty::Provided);
    if !maintainers.is_empty() {
        project.maintainers = Some(MetaProperty::Provided(maintainers));
    }
    if !info.topic.is_empty() {
        project.topics = Some(MetaProperty::Provided(info.topic.clone()));
    }
    if let Some(website) = website {
        project.urls = Some(MetaProperty::Provided(Urls {
            website: Some(MetaProperty::Provided(website)),
            documentation: None,
            repository: None,
        }));
    }
    project.metamodel = metamodel.map(MetaProperty::Provided);

    report.into_result(ModelToml::Project(Box::new(ProjectManifest {
        project,
        inline_usages: usages,
    })))
}

/// The declared `publisher`, else the one in `options`; `[project]`
/// requires one.
fn migrate_publisher(
    info: &InterchangeProjectInfoRaw,
    options: &MigrateOptions,
    report: &mut Report,
) -> Option<IndexPublisher> {
    let raw = info.publisher.as_ref().or(options.publisher.as_ref());
    let Some(raw) = raw else {
        report.push(Diagnostic::error(
            "the project declares no `publisher` and none was given; `[project]` requires one",
        ));
        return None;
    };
    parse_or_report(IndexPublisher::parse(raw.clone()), "publisher", report)
}

/// model-toml.md `sources`: the union of `index`'s values and
/// `checksum`'s keys.
///
/// Not one call to [`Sources::new`]: it stops at the first bad entry and
/// refuses duplicates, and this union duplicates by construction --
/// `index` maps *symbol* to path, so a file exporting ten symbols
/// appears ten times.
fn migrate_sources(meta: &InterchangeProjectMetadataRaw, report: &mut Report) -> Option<Sources> {
    let mut paths: IndexSet<Utf8UnixPathBuf> = IndexSet::new();
    let mut bad_checksum_keys: Vec<String> = Vec::new();

    let mut add = |raw: &str, from_checksum: bool, report: &mut Report| {
        let path = match normalize_relative(raw) {
            Ok(path) => path,
            Err(error) => {
                report.push(Diagnostic::error(format!(
                    "`{raw}` cannot be listed in `sources`: {error}"
                )));
                return;
            }
        };
        if Language::guess_from_path(&path).is_none() {
            if from_checksum {
                bad_checksum_keys.push(raw.to_owned());
            } else {
                // One diagnostic per entry: `index` has no plural rule
                // of its own.
                report.push(Diagnostic::error(format!(
                    "`sources` entry `{raw}` is not a KerML (`.kerml`) or SysML (`.sysml`) \
                     file; `sources` lists model files only"
                )));
            }
            return;
        }
        paths.insert(path);
    };

    for value in meta.index.values() {
        add(value, false, report);
    }
    for key in meta.checksum.iter().flat_map(indexmap::IndexMap::keys) {
        add(key, true, report);
    }

    // One diagnostic naming every offending key, so a project with five
    // of them is told about five.
    if !bad_checksum_keys.is_empty() {
        let names = bad_checksum_keys
            .iter()
            .map(|key| format!("`{key}`"))
            .collect::<Vec<_>>()
            .join(", ");
        report.push(Diagnostic::error(format!(
            "`checksum` lists {names}, which are not KerML or SysML files and cannot be \
             listed in `sources`; run `sysand exclude` on them before migrating"
        )));
    }

    match Sources::new(paths.into_iter().collect()) {
        Ok(sources) => Some(sources),
        Err(error) => {
            report.push(Diagnostic::error(error.to_string()));
            None
        }
    }
}

/// `metamodel` must be a valid IRI, with a well-formed release if it is a
/// recognized one, even though an unrecognized value is only a warning:
/// [`ProjectTable::validate`] refuses the others, so writing one would
/// produce a manifest this crate cannot read back.
fn migrate_metamodel(meta: &InterchangeProjectMetadataRaw, report: &mut Report) -> Option<String> {
    let raw = meta.metamodel.as_ref()?;
    let metamodel = match Metamodel::parse(raw) {
        Ok(metamodel) => metamodel,
        Err(error) => {
            report.push(Diagnostic::error(format!(
                "`metamodel` is `{raw}`, which is not a metamodel: {error}"
            )));
            return None;
        }
    };
    if let Metamodel::Other(_) = metamodel {
        report.push(Diagnostic::warning(
            ReadWarning::UnknownMetamodel {
                location: crate::manifest::diagnostics::KeyLocation::File(
                    crate::model::META_JSON_FILE.to_owned(),
                ),
                value: raw.clone(),
            }
            .to_string(),
        ));
    }
    Some(raw.clone())
}

/// model-toml.md "JSON to TOML": the metadata keys that do not survive.
/// `index` and `checksum` are named apart from their paths, which become
/// `sources`.
fn warn_discarded(meta: &InterchangeProjectMetadataRaw, report: &mut Report) {
    report.push(Diagnostic::warning(format!(
        "`created` (`{}`) is not migrated: a `model.toml` project has no creation time, and \
         `sysand build` records the build's own",
        meta.created
    )));
    if !meta.index.is_empty() {
        report.push(Diagnostic::warning(
            "`index` is not migrated apart from its paths, which become `sources`: the symbol \
             index is computed when the project is built",
        ));
    }
    if meta.checksum.as_ref().is_some_and(|c| !c.is_empty()) {
        report.push(Diagnostic::warning(
            "`checksum` is not migrated apart from its paths, which become `sources`: \
             checksums are computed when the project is built",
        ));
    }
    if meta.includes_derived.is_some() {
        report.push(Diagnostic::warning(
            "`includesDerived` is not migrated: `model.toml` does not carry it",
        ));
    }
    if meta.includes_implied.is_some() {
        report.push(Diagnostic::warning(
            "`includesImplied` is not migrated: `model.toml` does not carry it",
        ));
    }
}

fn migrate_usages(info: &InterchangeProjectInfoRaw, report: &mut Report) -> Vec<MemberUsage> {
    let mut usages = Vec::new();
    for usage in &info.usage {
        if let Some(converted) = migrate_usage(usage, report) {
            usages.push(MemberUsage::Declared(converted));
        }
    }
    usages
}

fn migrate_usage(usage: &InterchangeProjectUsageRaw, report: &mut Report) -> Option<Usage> {
    match usage {
        InterchangeProjectUsageRaw::Directory {
            dir,
            publisher,
            name,
        } => Some(Usage::SourcePath {
            publisher: parse_or_report(
                IndexPublisher::parse(publisher.clone()),
                "publisher",
                report,
            )?,
            name: parse_or_report(IndexName::parse(name.clone()), "name", report)?,
            dir: local_path(dir, RelativePathKind::Directory, report)?,
        }),
        InterchangeProjectUsageRaw::KparPath {
            kpar_path,
            publisher,
            name,
        } => Some(Usage::KparPath {
            publisher: parse_or_report(
                IndexPublisher::parse(publisher.clone()),
                "publisher",
                report,
            )?,
            name: parse_or_report(IndexName::parse(name.clone()), "name", report)?,
            kpar_path: local_path(kpar_path, RelativePathKind::File, report)?,
        }),
        InterchangeProjectUsageRaw::Resource {
            resource,
            version_constraint,
        } => migrate_resource(resource, version_constraint.as_deref(), report),
    }
}

/// A `resource` usage, by scheme.
///
/// The scheme is matched as a string: `utils::scheme`'s constants are
/// behind the `filesystem` and `networking` features and this module has
/// neither.
fn migrate_resource(
    resource: &str,
    version_constraint: Option<&str>,
    report: &mut Report,
) -> Option<Usage> {
    let scheme = resource.split_once(':').map(|(scheme, _)| scheme);

    match scheme {
        Some("pkg") => {
            let Ok(Some((publisher, name))) = parse_sysand_purl(resource) else {
                report.push(Diagnostic::error(format!(
                    "usage `{resource}` is not a `pkg:sysand` package URL; `model.toml` \
                     declares index usages by publisher and name"
                )));
                return None;
            };
            {
                let Some(raw) = version_constraint else {
                    // Needs the index: the pinned `^<version>` comes
                    // from resolving the usage.
                    report.push(needs_resolution(resource));
                    return None;
                };
                let constraint = match VersionReq::parse(raw) {
                    Ok(constraint) => constraint,
                    Err(error) => {
                        report.push(Diagnostic::error(format!(
                            "usage `{resource}` has version constraint `{raw}`, which is not a \
                             valid requirement: {error}"
                        )));
                        return None;
                    }
                };
                Some(Usage::Index {
                    publisher: parse_or_report(
                        IndexPublisher::parse(publisher.to_owned()),
                        "publisher",
                        report,
                    )?,
                    name: parse_or_report(IndexName::parse(name.to_owned()), "name", report)?,
                    version_constraint: constraint,
                })
            }
        }
        // The publisher and name come from the target, which means
        // reading it.
        Some("http" | "https" | "file") => {
            report.push(needs_resolution(resource));
            None
        }
        Some("ssh" | "git" | "git+http" | "git+https" | "git+ssh" | "git+file") => {
            report.push(Diagnostic::error(format!(
                "usage `{resource}` is a git repository, which cannot be declared in a \
                 `model.toml` of manifest version 1"
            )));
            None
        }
        _ => {
            report.push(Diagnostic::error(format!(
                "usage `{resource}` has no scheme `sysand migrate` understands"
            )));
            None
        }
    }
}

/// Deferred to the slice that adds the resolver. The text is a
/// placeholder in the sense that it describes a limitation rather than a
/// mistake, and it goes when the resolver arrives.
fn needs_resolution(resource: &str) -> Diagnostic {
    Diagnostic::error(format!(
        "usage `{resource}` names its target by location, so its publisher and name must be \
         read from the target; `sysand migrate` cannot do that yet"
    ))
}

/// A usage's local path, in the one form a `model.toml` admits.
///
/// [`parse_relative_unix_path`], not `normalize_relative`: a `dir` usage
/// of `../dep` is the ordinary shape of a dependency beside the project,
/// and normalization refuses a path that leaves its root. What it admits
/// is then written normalized, as model-toml.md "Usages" requires: a json
/// `./dep/` becomes `dep`.
fn local_path(raw: &str, kind: RelativePathKind, report: &mut Report) -> Option<Utf8UnixPathBuf> {
    match parse_relative_unix_path(raw, kind) {
        Ok(_) => {
            let normalized = crate::manifest::usage::normalize_local_path(raw);
            if normalized.is_none() {
                report.push(Diagnostic::error(format!(
                    "usage path `{raw}` names the project itself, which cannot use itself"
                )));
            }
            normalized
        }
        Err(error) => {
            report.push(Diagnostic::error(format!(
                "usage path `{raw}` is not usable in a `model.toml`: {error}"
            )));
            None
        }
    }
}

fn parse_or_report<T, E: std::fmt::Display>(
    parsed: Result<T, E>,
    key: &str,
    report: &mut Report,
) -> Option<T> {
    match parsed {
        Ok(value) => Some(value),
        Err(error) => {
            report.push(Diagnostic::error(format!("`{key}`: {error}")));
            None
        }
    }
}

#[cfg(test)]
#[path = "./migrate_tests.rs"]
mod tests;
