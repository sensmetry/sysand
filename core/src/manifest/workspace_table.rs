// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The `[workspace]` table of a `model.toml`: which projects the
//! workspace includes, and the values its members may inherit.
//!
//! The table lists the members and doubles as the implicit default
//! group, whose keys are written directly in `[workspace]`, alongside
//! `members` and `exclude`; named groups are
//! `[workspace.group.<group-name>]`.
//!
//! A workspace never inherits: there is nothing above it, so every value
//! here is given directly. [`WorkspaceTable::group_properties`] serves the
//! groups to `[project]` once markers are resolved.

use std::collections::BTreeMap;

use fluent_uri::Iri;
use semver::Version;
use spdx::Expression;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::manifest::common::{InheritFrom, Maintainer, ResolvedUrls, is_valid_group_name};
use crate::manifest::diagnostics::{Diagnostic, Diagnostics, Position, Validated};
use crate::manifest::reader::{self, TableReader, checked, maintainers, parsed};
use crate::manifest::usage::{Usage, UsageRaw, UsageSite, check_duplicates};
use crate::model::{IndexPublisher, KnownMetamodel};
use crate::utils::{NormalizeRelativeError, normalize_relative};

/// The keys a group gives, the default group and a named one alike.
///
/// `sources` is here although a group may not carry it: the reader has to
/// recognize the key to report the error.
///
/// `stdlib`, `manifest-version`, `name`, `description` and `root` are
/// absent: they belong to one project, and in a workspace are unknown
/// keys like any other (a warning).
pub(crate) const GROUP_KEYS: &[&str] = &[
    "publisher",
    "version",
    "license",
    "topics",
    "metamodel",
    "maintainers",
    "sources",
    "urls",
    "inline-usages",
];

/// The keys of `[workspace]` itself, which is the default group plus the
/// membership lists and the named groups.
const WORKSPACE_KEYS: &[&str] = &["members", "exclude", "group"];

/// The keys `urls` admits.
const URL_KEYS: &[&str] = &["website", "documentation", "repository"];

/// The values a group provides for members to inherit.
///
/// Every field is an `Option`: a group that gives `topics = []` satisfies
/// `topics.workspace = true`, and a group that gives no `topics` does not.
#[derive(Clone, Debug, Default)]
pub struct GroupProperties {
    pub publisher: Option<IndexPublisher>,
    pub version: Option<Version>,
    pub license: Option<Expression>,
    pub topics: Option<Vec<String>>,
    pub metamodel: Option<KnownMetamodel>,
    pub maintainers: Option<Vec<Maintainer>>,
    pub urls: Option<ResolvedUrls>,
    /// `[[workspace.inline-usages]]` for the default group,
    /// `[[workspace.group.<group-name>.inline-usages]]` for a named one.
    /// Entries are plain [`Usage`]s: the directory their `dir`/`kpar-path`
    /// is relative to is not recorded here.
    pub inline_usages: Vec<Usage>,
}

/// One entry of `members` or `exclude`.
///
/// An entry is admitted only in its normalized form, so the two
/// spellings are always equal. Both are kept because they answer
/// different questions: every path comparison uses the typed one, while
/// `--member <path>` matches a `members` entry literally.
#[derive(Clone, Debug, Eq)]
pub struct MemberPath {
    pub raw: String,
    pub path: Utf8UnixPathBuf,
    /// Where the entry was written, for diagnostics about the directory
    /// raised after this table was read.
    ///
    /// `None` for an entry that was not read from a file, which is what a
    /// hand-built table has.
    pub at: Option<Position>,
}

/// A path is where it points and how it was spelled. Where it was
/// *written* is not part of equality.
impl PartialEq for MemberPath {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw && self.path == other.path
    }
}

/// The `[workspace]` table, validated.
#[derive(Clone, Debug, Default)]
pub struct WorkspaceTable {
    pub members: Vec<MemberPath>,
    pub exclude: Vec<MemberPath>,
    /// The implicit default group: the keys written directly in
    /// `[workspace]`.
    pub default_group: GroupProperties,
    /// `[workspace.group.<group-name>]`.
    pub groups: BTreeMap<String, GroupProperties>,
}

/// Where a directory sits with respect to a workspace, as far as the
/// table alone can tell.
///
/// `Workspace::classify` adds an absolute path to relativize and the
/// loaded member to hand back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region<'a> {
    /// The index of the `members` entry, and the entry. A load that
    /// succeeded produced one member per entry, in order.
    Member(usize, &'a MemberPath),
    Excluded(&'a MemberPath),
    Unlisted,
}

impl WorkspaceTable {
    /// Which of the workspace's three regions `relative` falls in.
    ///
    /// `relative` must be relative to the workspace root and free of `.`
    /// and `..` components, as [`normalize_relative`] produces; this is
    /// not checked.
    ///
    /// The most specific entry answers, and an exact tie between the two
    /// lists goes to `members`.
    pub fn region(&self, relative: &Utf8UnixPath) -> Region<'_> {
        // At most one member can match: nested members are refused.
        let member = self.members.iter().enumerate().find_map(|(index, member)| {
            prefix_depth(relative, &member.path).map(|at| (at, index, member))
        });
        let excluded = self.excluding(relative);

        match (member, excluded) {
            (Some((member_at, index, member)), Some((excluded_at, _)))
                if member_at >= excluded_at =>
            {
                Region::Member(index, member)
            }
            (_, Some((_, excluded))) => Region::Excluded(excluded),
            (Some((_, index, member)), None) => Region::Member(index, member),
            (None, None) => Region::Unlisted,
        }
    }

    /// The `exclude` entry that covers `relative`, and how many of its
    /// segments matched.
    ///
    /// Anything under an `exclude` entry is outside the workspace.
    /// Entries may nest, so the most specific one answers.
    pub fn excluding(&self, relative: &Utf8UnixPath) -> Option<(usize, &MemberPath)> {
        self.exclude
            .iter()
            .filter_map(|excluded| prefix_depth(relative, &excluded.path).map(|at| (at, excluded)))
            .max_by_key(|(at, _)| *at)
    }

    /// Read and check the contents of a `[workspace]` table.
    ///
    /// The caller has already found the table in the file.
    pub fn validate(reader: &TableReader<'_>) -> Result<Validated<Self>, Diagnostics> {
        let mut report = Diagnostics::new();

        let members = member_paths(reader, "members", true, &mut report);
        let exclude = member_paths(reader, "exclude", false, &mut report);
        let members = cross_list_conflicts(members, &exclude, &mut report);

        let mut known: Vec<&str> = GROUP_KEYS.to_vec();
        known.extend_from_slice(WORKSPACE_KEYS);
        let default_group = read_group(reader, "workspace", &known, &mut report);

        let mut groups = BTreeMap::new();
        for (name, group) in reader.named_tables("group", &mut report) {
            if !is_valid_group_name(name) {
                // Not read further: no marker can name it.
                report.push(
                    Diagnostic::error(format!(
                        "`{name}` is not a valid workspace group name (a bare TOML key: \
                         letters, digits, `-` and `_`)"
                    ))
                    .at_opt(group.here()),
                );
                continue;
            }
            let table = format!("workspace.group.{name}");
            let properties = read_group(&group, &table, GROUP_KEYS, &mut report);
            groups.insert(name.to_owned(), properties);
        }

        report.sort_by_position();
        report.into_result(Self {
            members,
            exclude,
            default_group,
            groups,
        })
    }

    /// The group a `<key>.workspace` marker points at, if the workspace
    /// defines it.
    pub fn group_properties(&self, from: &InheritFrom) -> Option<&GroupProperties> {
        match from {
            InheritFrom::DefaultGroup => Some(&self.default_group),
            InheritFrom::Group(group) => self.groups.get(group),
        }
    }
}

/// Read one group: the default group out of `[workspace]` itself, or a
/// named one out of its own table. `table` is the dotted name used in
/// messages; `known` is the set of keys that are not unknown here.
fn read_group(
    reader: &TableReader<'_>,
    table: &str,
    known: &[&str],
    report: &mut Diagnostics,
) -> GroupProperties {
    // Recognized only to be refused, in the default group and named ones.
    if reader.has("sources") {
        report.push(
            Diagnostic::error(
                "`sources` cannot appear in `[workspace]` or in a group; it is per-project only",
            )
            .at_opt(reader.at("sources")),
        );
    }

    let publisher = reader
        .string("publisher", report)
        .and_then(|raw| parsed(IndexPublisher::parse(raw), reader, "publisher", report));
    let version = reader.string("version", report).and_then(|raw| {
        checked(
            Version::parse(raw),
            reader,
            "version",
            raw,
            "a valid version",
            report,
        )
    });
    let license = reader.string("license", report).and_then(|raw| {
        checked(
            Expression::parse(raw),
            reader,
            "license",
            raw,
            "a valid SPDX expression",
            report,
        )
    });
    let topics = reader
        .strings("topics", report)
        .map(|values| values.into_iter().map(ToOwned::to_owned).collect());
    let metamodel = reader::metamodel(reader, report);
    let maintainers = maintainers(reader, report);
    let urls = urls(reader, report);
    let inline_usages = group_usages(reader, table, report);

    report.extend(reader.unknown_keys(known, None));

    GroupProperties {
        publisher,
        version,
        license,
        topics,
        metamodel,
        maintainers,
        urls,
        inline_usages,
    }
}

fn urls(reader: &TableReader<'_>, report: &mut Diagnostics) -> Option<ResolvedUrls> {
    let urls = reader.table("urls", report)?;
    let url = |key: &str, report: &mut Diagnostics| {
        let raw = urls.string(key, report)?;
        checked(
            Iri::parse(raw.to_owned()).map_err(|(error, _)| error),
            &urls,
            key,
            raw,
            "a valid URL",
            report,
        )
    };
    let website = url("website", report);
    let documentation = url("documentation", report);
    let repository = url("repository", report);
    report.extend(urls.unknown_keys(URL_KEYS, None));
    Some(ResolvedUrls {
        website,
        documentation,
        repository,
    })
}

/// A group's own entries, which have nothing above them to inherit from.
fn group_usages(reader: &TableReader<'_>, table: &str, report: &mut Diagnostics) -> Vec<Usage> {
    let array = format!("{table}.inline-usages");
    let mut entries = Vec::new();
    let tables = reader.tables("inline-usages", report);
    for (index, entry) in tables.iter().enumerate() {
        let site = UsageSite {
            array: &array,
            index,
            entry,
        };
        match UsageRaw::from_table(entry.raw()).validate_declared(&site) {
            Ok(usage) => entries.push((site, usage)),
            Err(problems) => report.extend(problems.into_diagnostics()),
        }
    }
    // Duplicates are counted per array; only validated entries are compared.
    report.extend(check_duplicates(&entries));
    entries.into_iter().map(|(_, usage)| usage).collect()
}

/// The `members` (or `exclude`) entries, as paths relative to the
/// workspace root.
///
/// An entry is taken exactly as written, as a `sources` entry is: it is
/// refused unless it already is the one form [`normalize_relative`]
/// would produce. A path that leaves the root or is the root still gets
/// the message that says so.
///
/// `cross_entry` enables the rules between entries (no entry inside
/// another, none listed twice). They apply to `members` only: nested
/// `exclude` entries are harmless, as the inner one excludes a subtree
/// of the outer.
fn member_paths(
    reader: &TableReader<'_>,
    key: &str,
    cross_entry: bool,
    report: &mut Diagnostics,
) -> Vec<MemberPath> {
    let Some(raw) = reader.strings(key, report) else {
        return Vec::new();
    };
    let mut paths: Vec<MemberPath> = Vec::with_capacity(raw.len());
    for (index, entry) in raw.iter().enumerate() {
        let at = || reader.at_element(key, index);
        let path = match normalize_relative(entry) {
            Ok(path) if path.as_str() == *entry => path,
            Ok(_) => {
                report.push(
                    Diagnostic::error(format!(
                        "`{key}` path `{entry}` is not in the form `{key}` takes: paths are \
                         relative to the workspace root, `/`-separated,\nand carry no `.` or \
                         `..` component and no repeated, leading or trailing slash"
                    ))
                    .at_opt(at()),
                );
                continue;
            }
            Err(error) => {
                report.push(Diagnostic::error(path_message(key, entry, &error)).at_opt(at()));
                continue;
            }
        };
        if cross_entry {
            if paths.iter().any(|other| other.path == path) {
                report.push(
                    Diagnostic::error(format!("`{key}` path `{entry}` is listed twice"))
                        .at_opt(at()),
                );
                continue;
            }
            if let Some((outer, inner)) = paths.iter().find_map(|other| {
                if path.starts_with(&other.path) {
                    Some((&other.path, &path))
                } else if other.path.starts_with(&path) {
                    Some((&path, &other.path))
                } else {
                    None
                }
            }) {
                report.push(
                    Diagnostic::error(format!(
                        "`{key}` path `{inner}` lies inside member `{outer}`; nested members \
                         are not supported"
                    ))
                    .at_opt(at()),
                );
                continue;
            }
        }
        paths.push(MemberPath {
            raw: (*entry).to_owned(),
            path,
            at: at(),
        });
    }
    paths
}

/// A `members` entry at or under an `exclude` entry is refused.
///
/// The two lists say opposite things about the same directory and neither
/// takes precedence, so the combination is reported as a conflict.
///
/// The offending entries are dropped from `members`; the report is what
/// stops the table being used.
fn cross_list_conflicts(
    members: Vec<MemberPath>,
    exclude: &[MemberPath],
    report: &mut Diagnostics,
) -> Vec<MemberPath> {
    members
        .into_iter()
        .filter(|member| {
            let Some(excluded) = exclude
                .iter()
                .find(|excluded| member.path.starts_with(&excluded.path))
            else {
                return true;
            };
            let message = if member.path == excluded.path {
                format!(
                    "`members` path `{}` is also in `exclude`; a directory is either a member \
                     or outside the workspace",
                    member.raw
                )
            } else {
                format!(
                    "`members` path `{}` lies under `exclude` path `{}`; everything under an \
                     excluded directory is outside the workspace",
                    member.raw, excluded.raw
                )
            };
            report.push(Diagnostic::error(message).at_opt(member.at.clone()));
            false
        })
        .collect()
}

/// How many segments of `entry` `path` is under, or `None` if it is not.
///
/// The workspace root itself is never an entry, so a depth of zero cannot
/// arise.
pub(crate) fn prefix_depth(path: &Utf8UnixPath, entry: &Utf8UnixPath) -> Option<usize> {
    path.starts_with(entry).then(|| entry.components().count())
}

/// The message for a path that is not in normalizable form.
fn path_message(key: &str, raw: &str, error: &NormalizeRelativeError) -> String {
    match error {
        NormalizeRelativeError::Absolute { path: _ } => format!(
            "`{key}` path `{raw}` is absolute; `members` and `exclude` are relative to the \
             workspace root"
        ),
        NormalizeRelativeError::ContainsBackslash { path: _ } => format!(
            "`{key}` path `{raw}` contains a backslash `\\`;\n\
             entries are `/`-separated, so that a manifest reads the same way\n\
             on every operating system"
        ),
        NormalizeRelativeError::ContainsNull { path: _ } => {
            format!("`{key}` path `{raw}` contains a NULL character")
        }
        NormalizeRelativeError::EscapesRoot { path: _ } => {
            format!("`{key}` path `{raw}` escapes the workspace root")
        }
        NormalizeRelativeError::IsRoot { path: _ } => {
            format!("`{key}` path `{raw}` is the workspace root, which must not contain a project")
        }
    }
}

#[cfg(test)]
#[path = "./workspace_table_tests.rs"]
mod tests;
