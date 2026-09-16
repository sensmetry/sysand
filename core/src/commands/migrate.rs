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
        usage::{MemberUsage, Usage, UsageEntry as _},
    },
    model::{
        IndexName, IndexPublisher, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsageRaw, KnownMetamodel,
    },
    purl::parse_sysand_purl,
    symbols::Language,
    utils::{RelativePathKind, normalize_relative, parse_relative_unix_path},
};

/// How a `resource` usage that names its target by location is turned
/// into a usage.
///
/// A callback, and it returns the *finished* usage, because every step
/// is a filesystem question: which of `file:`'s two project kinds the
/// target is, what its publisher and name are, and what the path looks
/// like relative to the project being migrated. Keeping them here would
/// make this module impure and, for the URL conversion, unbuildable --
/// `url::Url::to_file_path` is `cfg`'d off on the wasm target
/// `bindings/js` builds.
pub type TargetLookup<'a> = &'a dyn Fn(&str) -> Result<Usage, String>;

/// What the caller supplies that the documents do not.
#[derive(Clone, Debug, Default)]
pub struct MigrateOptions {
    /// `--publisher`. Used when the json project declares none;
    /// model-toml.md gives the file's own value "the same treatment as
    /// `name`", so a declared publisher is migrated rather than
    /// overridden.
    pub publisher: Option<String>,
    /// `--metamodel`. model-toml.md "JSON to TOML": used "when the json
    /// side has none or one this Sysand does not recognize". Unlike
    /// `--publisher` it therefore also replaces a declared value -- one
    /// `model.toml` cannot hold is no more usable than none at all.
    pub metamodel: Option<String>,
}

/// Convert a json project's documents into a `model.toml` manifest.
///
/// All-or-nothing: every problem is collected, so a project with three
/// mistakes is reported once rather than three times.
pub fn migrate_project(
    info: &InterchangeProjectInfoRaw,
    meta: &InterchangeProjectMetadataRaw,
    options: &MigrateOptions,
    target: TargetLookup<'_>,
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
    let usages = migrate_usages(info, target, &mut report);
    check_duplicate_usages(&usages, &mut report);
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
    let metamodel = migrate_metamodel(meta, options, &mut report);

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

/// Appendix A2 row 52.
///
/// model-toml.md "JSON to TOML": "`metamodel`: use as-is when it is one
/// of the four recognized values. A json project may carry none, or one
/// this Sysand does not recognize, and `model.toml` admits neither: the
/// command then errors before writing, unless `--metamodel <url>`
/// supplies a recognized value".
///
/// So this is *not* `migrate_publisher`'s rule, which keeps a declared
/// value and falls back only when there is none: a metamodel
/// `model.toml` cannot hold is no more usable than an absent one, and
/// the flag replaces it. There is no separate IRI check -- the four
/// values are IRIs, so one that parses as a metamodel is one, and a
/// second message about the same key would only crowd the report.
fn migrate_metamodel(
    meta: &InterchangeProjectMetadataRaw,
    options: &MigrateOptions,
    report: &mut Report,
) -> Option<KnownMetamodel> {
    let declared = meta.metamodel.as_ref();
    if let Some(metamodel) = declared.and_then(|raw| KnownMetamodel::recognize(raw)) {
        return Some(metamodel);
    }
    let Some(supplied) = options.metamodel.as_ref() else {
        report.push(Diagnostic::error(match declared {
            Some(raw) => format!(
                "the project's `.meta.json` declares `{raw}`, which is not a metamodel \
                 `model.toml` recognizes;\npass `--metamodel <url>`, one of {list}",
                list = KnownMetamodel::RECOGNIZED
            ),
            None => format!(
                "the project's `.meta.json` declares no `metamodel`, which `model.toml` \
                 requires;\npass `--metamodel <url>`, one of {list}",
                list = KnownMetamodel::RECOGNIZED
            ),
        }));
        return None;
    };
    let Some(metamodel) = KnownMetamodel::recognize(supplied) else {
        report.push(Diagnostic::error(format!(
            "`{supplied}` is not a metamodel Sysand recognizes;\nit is one of {list}",
            list = KnownMetamodel::RECOGNIZED
        )));
        return None;
    };
    // What is being dropped is worth saying, even though the fix is
    // already in hand.
    if let Some(raw) = declared {
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
    Some(metamodel)
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

/// The rule `check_duplicates` (`manifest::usage`) applies on the read
/// path, applied here on the write path.
///
/// It cannot be called: its signature takes usage *sites* and a span
/// index, and a manifest built from values has neither. So the rule and
/// its wording are reused rather than the function -- a user who fixes
/// the json and re-runs should not meet a second phrasing of one rule.
///
/// Pairwise, as `check_duplicates` is: three colliding entries produce
/// two diagnostics, the same as reading such a file back.
///
/// Without this, `migrate` writes a `model.toml` that `sysand` itself
/// refuses to read -- after deleting the json manifests that were the
/// only readable copy.
fn check_duplicate_usages(usages: &[MemberUsage], report: &mut Report) {
    for (index, usage) in usages.iter().enumerate() {
        let identity = usage.identity();
        let Some(first) = usages[..index]
            .iter()
            .find(|earlier| earlier.identity() == identity)
        else {
            continue;
        };
        report.push(Diagnostic::error(format!(
            "usages `{}` and `{}` name the same project; two entries whose publisher and \
             name normalize to the same value are a duplicate",
            first.label(),
            usage.label()
        )));
    }
}

fn migrate_usages(
    info: &InterchangeProjectInfoRaw,
    target: TargetLookup<'_>,
    report: &mut Report,
) -> Vec<MemberUsage> {
    let mut usages = Vec::new();
    for usage in &info.usage {
        if let Some(converted) = migrate_usage(usage, target, report) {
            usages.push(MemberUsage::Declared(converted));
        }
    }
    usages
}

fn migrate_usage(
    usage: &InterchangeProjectUsageRaw,
    target: TargetLookup<'_>,
    report: &mut Report,
) -> Option<Usage> {
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
        } => migrate_resource(resource, version_constraint.as_deref(), target, report),
        // A typed index usage already carries everything a manifest
        // entry needs
        InterchangeProjectUsageRaw::Index {
            publisher,
            name,
            version_constraint,
        } => {
            let constraint = match VersionReq::parse(version_constraint) {
                Ok(constraint) => constraint,
                Err(error) => {
                    report.push(Diagnostic::error(format!(
                        "usage `{publisher}/{name}` has version constraint `{version_constraint}`, \
                         which is not a valid requirement: {error}"
                    )));
                    return None;
                }
            };
            Some(Usage::Index {
                publisher: parse_or_report(
                    IndexPublisher::parse(publisher.clone()),
                    "publisher",
                    report,
                )?,
                name: parse_or_report(IndexName::parse(name.clone()), "name", report)?,
                version_constraint: constraint,
            })
        }
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
    target: TargetLookup<'_>,
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
                    // model-toml.md: "resolved at migration time and
                    // pinned to `^<version>`, where `<version>` is the
                    // version that resolution found". Which version
                    // that is belongs to the caller, with the index.
                    return match target(resource) {
                        Ok(usage) => Some(usage),
                        Err(reason) => {
                            report.push(Diagnostic::error(reason));
                            None
                        }
                    };
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
        // model-toml.md: "if `file:`, translate into kpar/source path;
        // the publisher and name come from the target, and a target
        // without a publisher is an error".
        Some("file") => match target(resource) {
            Ok(usage) => Some(usage),
            Err(reason) => {
                report.push(Diagnostic::error(reason));
                None
            }
        },
        // model-toml.md: "use as-is for URL, discard `versionConstraint`
        // if present with a warning". The warning has to be here rather
        // than in the caller's lookup: the callback is handed only the
        // resource string and never sees the constraint.
        Some("http" | "https") => {
            if let Some(constraint) = version_constraint {
                report.push(Diagnostic::warning(format!(
                    "usage `{resource}` has version constraint `{constraint}`, which is not \
                     migrated: a `url` usage carries no version constraint"
                )));
            }
            // model-toml.md: "use as-is for URL … the publisher and
            // name come from the target, which is fetched".
            match target(resource) {
                Ok(usage) => Some(usage),
                Err(reason) => {
                    report.push(Diagnostic::error(reason));
                    None
                }
            }
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

/// Whether the project's directory has uncommitted work.
///
/// `migrate` deletes the json manifests, so the user should have a way
/// back.
#[cfg(feature = "networking")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VcsState {
    Clean,
    /// The paths that differ, relative to the repository's work tree.
    Dirty(Vec<String>),
    NoRepository,
}

#[cfg(feature = "networking")]
#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    #[error("failed to open the git repository at `{path}`: {source}")]
    Open {
        path: camino::Utf8PathBuf,
        #[source]
        source: Box<gix::Error>,
    },
    #[error("failed to read the status of the git repository at `{path}`: {reason}")]
    Status {
        path: camino::Utf8PathBuf,
        reason: String,
    },
}

/// The state of the work tree *at `root`*.
#[cfg(feature = "networking")]
pub fn check_vcs(root: &camino::Utf8Path) -> Result<VcsState, VcsError> {
    use gix::{
        bstr::{BString, ByteSlice as _},
        dir::entry::Status as DirStatus,
        status::{
            Item, UntrackedFiles, index_worktree::Item as IndexWorktreeItem,
            plumbing::index_as_worktree::EntryStatus,
        },
    };

    // `discover`, not `open`: a project in a subdirectory of a
    // repository is inside its work tree.
    let repo = match gix::discover(root) {
        Ok(repo) => repo,
        Err(error) => {
            // Only three of the discovery variants mean "there is no
            // repository here". Folding the rest in would let
            // `--allow-no-vcs` hide a broken one. gix reports one error
            // type for everything, with the cause inside it.
            if matches!(
                error.downcast_any_ref::<gix::discover::upwards::Error>(),
                Some(
                    gix::discover::upwards::Error::NoGitRepository { path: _ }
                        | gix::discover::upwards::Error::NoGitRepositoryWithinCeiling {
                            path: _,
                            ceiling_height: _
                        }
                        | gix::discover::upwards::Error::NoGitRepositoryWithinFs {
                            path: _,
                            limit: _
                        }
                )
            ) {
                return Ok(VcsState::NoRepository);
            }
            return Err(VcsError::Open {
                path: root.to_owned(),
                source: Box::new(error),
            });
        }
    };

    let fail = |reason: String| VcsError::Status {
        path: root.to_owned(),
        reason,
    };

    // Scope the walk to the project. Without a pathspec the status
    // covers the whole work tree, so a monorepo with one unrelated
    // modified file would refuse every migration in it.
    let workdir = repo
        .workdir()
        .ok_or_else(|| fail("the repository has no work tree".to_owned()))?;
    let workdir = camino::Utf8Path::from_path(workdir)
        .ok_or_else(|| fail("the repository's work tree is not valid UTF-8".to_owned()))?;
    let absolute = crate::project::utils::wrapfs::canonicalize(root)
        .map_err(|error| fail(error.to_string()))?;
    let workdir = crate::project::utils::wrapfs::canonicalize(workdir)
        .map_err(|error| fail(error.to_string()))?;
    let pathspec: Vec<BString> = match absolute.strip_prefix(&workdir) {
        Ok(relative) if !relative.as_str().is_empty() => vec![BString::from(relative.as_str())],
        // The project *is* the work tree root: no filter needed.
        _ => Vec::new(),
    };

    let status = repo
        .status(gix::progress::Discard)
        .map_err(|error| fail(error.to_string()))?
        // Explicit, because `Repository::status` otherwise takes
        // `status.showUntrackedFiles` from the user's configuration, so
        // a `no` there would silently stop untracked files counting.
        // `Files` rather than the `Collapsed` default so the report
        // names files rather than directories.
        .untracked_files(UntrackedFiles::Files)
        .into_iter(pathspec)
        .map_err(|error| fail(error.to_string()))?;

    let mut dirty = Vec::new();
    for item in status {
        let item = item.map_err(|error| fail(error.to_string()))?;
        let counts = match &item {
            // Staged.
            Item::TreeIndex(_) => true,
            Item::IndexWorktree(IndexWorktreeItem::Modification {
                status,
                entry: _,
                entry_index: _,
                rela_path: _,
            }) => {
                // `NeedsUpdate` is a stat-cache refresh hint and
                // `IntentToAdd` is documented as no change; counting
                // either gives a machine-dependent false `Dirty`.
                matches!(
                    status,
                    EntryStatus::Change(_)
                        | EntryStatus::Conflict {
                            summary: _,
                            entries: _
                        }
                )
            }
            Item::IndexWorktree(IndexWorktreeItem::DirectoryContents {
                entry,
                collapsed_directory_status: _,
            }) => entry.status == DirStatus::Untracked,
            Item::IndexWorktree(IndexWorktreeItem::Rewrite {
                source: _,
                dirwalk_entry: _,
                dirwalk_entry_collapsed_directory_status: _,
                dirwalk_entry_id: _,
                diff: _,
                copy: _,
            }) => true,
        };
        if counts {
            dirty.push(item.location().to_str_lossy().into_owned());
        }
    }

    if dirty.is_empty() {
        Ok(VcsState::Clean)
    } else {
        Ok(VcsState::Dirty(dirty))
    }
}
