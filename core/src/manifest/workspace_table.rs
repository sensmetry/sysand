// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The `[workspace]` table of a `model.toml`: which projects the
//! workspace includes, and the values its members may inherit.
//!
//! workspaces.md "Included projects": the table "lists the members and
//! doubles as the implicit default group", whose "keys are written
//! directly in `[workspace]`, alongside `members` and `exclude`"; named
//! groups are `[workspace.group.<group-name>]`.
//!
//! A workspace never inherits: there is nothing above it, so every value
//! here is given directly. That is what makes this the simpler of the two
//! tables, and why it comes before `[project]`, which
//! [`WorkspaceTable::group_properties`] serves once markers are resolved.

use std::collections::BTreeMap;

use fluent_uri::Iri;
use semver::Version;
use spdx::Expression;
use typed_path::Utf8UnixPathBuf;

use crate::manifest::common::{InheritFrom, Maintainer, ResolvedUrls, is_valid_group_name};
use crate::manifest::diagnostics::{Diagnostic, KeyLocation, ReadWarning, Report, Validated};
use crate::manifest::reader::{TableReader, checked, maintainers, parsed};
use crate::manifest::spans::Positions;
use crate::manifest::usage::{Usage, UsageRaw, UsageSite, check_duplicates};
use crate::model::{IndexPublisher, Metamodel};
use crate::utils::{NormalizeRelativeError, normalize_relative};

/// The keys a group gives, the default group and a named one alike.
///
/// `sources` is here although a group may not carry it: workspaces.md
/// makes it "an error that says so, rather than an unknown-key warning",
/// which it can only be if the reader recognizes the key.
///
/// `stdlib` and `manifest-version` are deliberately absent -- in a
/// workspace they are "an unknown key like any other (a warning)" -- and
/// so are `name`, `description` and `root`, which identify one project.
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

/// model-toml.md `urls`: "currently allowed keys: website, repository,
/// documentation".
const URL_KEYS: &[&str] = &["website", "documentation", "repository"];

/// workspaces.md "Common metadata" and "Common usages": the values a
/// group provides for members to inherit.
///
/// Every field is an `Option` because "the group does not give this key"
/// is what a marker pointing at it has to report, and is not the same as
/// a default: a group that gives `topics = []` satisfies a
/// `topics.workspace = true`, and a group that gives no `topics` does
/// not.
#[derive(Clone, Debug, Default)]
pub struct GroupProperties {
    pub publisher: Option<IndexPublisher>,
    pub version: Option<Version>,
    pub license: Option<Expression>,
    pub topics: Option<Vec<String>>,
    /// Kept as written: model-toml.md warns about a value it does not
    /// recognize rather than refusing it, so this is not an `Iri`.
    pub metamodel: Option<String>,
    pub maintainers: Option<Vec<Maintainer>>,
    pub urls: Option<ResolvedUrls>,
    /// `[[workspace.inline-usages]]` for the default group,
    /// `[[workspace.group.<group-name>.inline-usages]]` for a named one.
    /// A group entry is a plain [`Usage`]: there is nothing above a
    /// workspace to inherit from. Which directory its `dir`/`kpar-path`
    /// is relative to belongs to whoever reads the group, not to the
    /// usage, so these are not [`crate::manifest::usage::LocatedUsage`]s.
    pub inline_usages: Vec<Usage>,
}

/// One entry of `members` or `exclude`.
///
/// Both spellings are kept: every path comparison uses the normalized
/// one, while `--member <path>` matches a `members` entry "literally"
/// (workspaces.md "Command scopes").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberPath {
    pub raw: String,
    pub path: Utf8UnixPathBuf,
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

impl WorkspaceTable {
    /// Read and check the contents of a `[workspace]` table.
    ///
    /// Finding the table in the document, and refusing a file that also
    /// carries a `[project]`, belong to the reader of the file as a
    /// whole; what is fixed here is where the table sits, which is what
    /// addresses its keys in the span index.
    pub fn validate(
        table: &toml::Table,
        positions: &Positions<'_>,
    ) -> Result<Validated<Self>, Report> {
        let reader = TableReader::new(
            table,
            vec!["workspace".to_owned()],
            KeyLocation::Table("workspace".to_owned()),
            positions,
        );
        let mut report = Report::new();

        let members = member_paths(&reader, "members", true, &mut report);
        let exclude = member_paths(&reader, "exclude", false, &mut report);

        let mut known: Vec<&str> = GROUP_KEYS.to_vec();
        known.extend_from_slice(WORKSPACE_KEYS);
        let default_group = read_group(&reader, "workspace", &known, &mut report);

        let mut groups = BTreeMap::new();
        for (name, group) in reader.named_tables("group", &mut report) {
            if !is_valid_group_name(name) {
                // Not read further: a name a marker could never spell
                // cannot be reached, and a name holding a `.` would
                // address the wrong keys in the span index.
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
/// named one out of its own table. `table` is the dotted name the span
/// index and the messages use; `known` is what is not an unknown key
/// here.
fn read_group(
    reader: &TableReader<'_>,
    table: &str,
    known: &[&str],
    report: &mut Report,
) -> GroupProperties {
    // workspaces.md "Common metadata": `sources` "cannot appear in
    // `[workspace]` (neither in the default group nor in a named one)".
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
    let metamodel = reader.string("metamodel", report).and_then(|raw| {
        // model-toml.md "Metamodel" opens with "`metamodel` is a URL",
        // and only then says "Any other value is a warning, not an
        // error": the other values are the other URLs. See
        // `ProjectTable::validate`, which refuses a non-URL for the same
        // reasons.
        let metamodel = checked(
            Metamodel::parse(raw),
            reader,
            "metamodel",
            raw,
            "a metamodel",
            report,
        )?;
        if let Metamodel::Other(_) = metamodel {
            report.push(
                Diagnostic::from(ReadWarning::UnknownMetamodel {
                    location: reader.location().clone(),
                    value: raw.to_owned(),
                })
                .at_opt(reader.at("metamodel")),
            );
        }
        Some(raw.to_owned())
    });
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

fn urls(reader: &TableReader<'_>, report: &mut Report) -> Option<ResolvedUrls> {
    let urls = reader.table("urls", report)?;
    let url = |key: &str, report: &mut Report| {
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

/// workspaces.md "Common usages": a group's own entries, which have
/// nothing above them to inherit from.
fn group_usages(reader: &TableReader<'_>, table: &str, report: &mut Report) -> Vec<Usage> {
    let array = format!("{table}.inline-usages");
    let mut entries = Vec::new();
    for (index, entry) in reader.tables("inline-usages", report).iter().enumerate() {
        let site = UsageSite {
            array: &array,
            index,
        };
        match UsageRaw::from_table(entry.raw()).validate_declared(&site, reader.positions()) {
            Ok(usage) => entries.push((site, usage)),
            Err(problems) => report.extend(problems.into_diagnostics()),
        }
    }
    // model-toml.md "Normalization": duplicates are counted per array,
    // and only the entries that validated can be compared.
    report.extend(check_duplicates(&entries, reader.positions()));
    entries.into_iter().map(|(_, usage)| usage).collect()
}

/// workspaces.md "Included projects": the `members` (or `exclude`)
/// entries, as paths relative to the workspace root.
///
/// An entry is taken exactly as written, as a `sources` entry is: it is
/// refused unless it already is the one form [`normalize_relative`]
/// would produce, rather than normalized into it. A path that leaves the
/// root or is the root still gets the message that says so, which is
/// more use than "not in normal form" for `..` or `.`.
///
/// `cross_entry` is the rules that hold between entries -- no entry
/// inside another, no entry listed twice -- which workspaces.md states
/// for `members` only: `exclude` gets "the same path rules ... (relative,
/// below the root, and never the root itself)", and two excluded
/// directories where one contains the other exclude the same tree.
fn member_paths(
    reader: &TableReader<'_>,
    key: &str,
    cross_entry: bool,
    report: &mut Report,
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
        });
    }
    paths
}

/// The three conditions that are about how a path is
/// written, rather than where it points, read as `sources` reads them.
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
