// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::manifest::diagnostics::{Diagnostic, Report, Validated};
use crate::manifest::spans::Source;
use crate::manifest::usage::Usage;
use crate::manifest::workspace_table::WorkspaceTable;
use crate::model::KnownMetamodel;

/// The whole `[workspace]` of a document, read the way `ModelToml::parse`
/// will read it.
fn read(text: &str) -> Result<Validated<WorkspaceTable>, Report> {
    let (table, positions) = Source::new("model.toml", text)
        .read()
        .expect("the test document parses");
    let workspace = table
        .get("workspace")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();
    WorkspaceTable::validate(&workspace, &positions)
}

/// A `[workspace]` whose body is `body`, so that its keys start on line 2.
fn one(body: &str) -> String {
    format!("[workspace]\n{body}")
}

fn accepted(body: &str) -> Validated<WorkspaceTable> {
    read(&one(body)).expect("the table validates")
}

fn table(body: &str) -> WorkspaceTable {
    accepted(body).into_parts().0
}

fn rejected(body: &str) -> Report {
    read(&one(body)).expect_err("the table is rejected")
}

fn messages(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect()
}

fn texts(report: &Report) -> Vec<&str> {
    messages(report.diagnostics())
}

/// Whether every diagnostic sits strictly after the one before it, which
/// is what "in document order" means for a report.
fn is_ascending(places: &[(usize, usize)]) -> bool {
    places.windows(2).all(|pair| match pair {
        [earlier, later] => earlier < later,
        _ => true,
    })
}

fn places(report: &Report) -> Vec<(usize, usize)> {
    report
        .diagnostics()
        .iter()
        .map(|diagnostic| {
            let position = diagnostic.position.as_ref().expect("positioned");
            (position.line, position.column)
        })
        .collect()
}

/// Every key workspaces.md "Common metadata" lists, as the specification
/// writes them. Ported from the prototype's `FULL_WORKSPACE_TOML`.
const FULL: &str = "\
manifest-version = \"1.0.0\"

[workspace]
members = [\"projects/a\", \"projects/b/subfolder1\"]
exclude = [\"vendor\"]
publisher = \"acme\"
version = \"1.0.0\"
license = \"MIT\"
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
topics = [\"shared\"]
maintainers = [\"Root Maintainer <root@example.com>\"]

[workspace.urls]
website = \"https://example.com\"
repository = \"https://example.com/repo.git\"
documentation = \"https://example.com/docs\"

[[workspace.inline-usages]]
publisher = \"acme\"
name = \"shared-dep\"
version = \"^2.0\"

[workspace.group.legacy]
publisher = \"acme-legacy\"
version = \"0.9.0\"
license = \"Apache-2.0\"
metamodel = \"https://www.omg.org/spec/SysML/20240201\"
topics = [\"legacy\"]
maintainers = [\"Legacy Maintainer <legacy@example.com>\"]

[workspace.group.legacy.urls]
website = \"https://legacy.example.com\"

[[workspace.group.legacy.inline-usages]]
publisher = \"acme\"
name = \"legacy-dep\"
dir = \"legacy/dep\"
";

// ----------------------------------------------------------------------
// A group's values.

#[test]
fn a_group_reads_every_documented_key() {
    let validated = read(FULL).expect("the table validates");
    let (table, warnings) = validated.into_parts();
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let root = &table.default_group;
    assert_eq!(root.publisher.as_ref().expect("publisher").as_str(), "acme");
    assert_eq!(
        root.version,
        Some(semver::Version::parse("1.0.0").expect("semver"))
    );
    assert_eq!(root.license.as_ref().expect("license").to_string(), "MIT");
    assert_eq!(
        root.metamodel.as_ref().map(ToString::to_string).as_deref(),
        Some("https://www.omg.org/spec/SysML/20250201")
    );
    assert_eq!(
        root.topics.as_deref(),
        Some(["shared".to_owned()].as_slice())
    );
    assert_eq!(
        root.maintainers
            .as_ref()
            .expect("maintainers")
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["Root Maintainer <root@example.com>"]
    );
    let urls = root.urls.as_ref().expect("urls");
    assert_eq!(
        urls.website.as_ref().expect("website").as_str(),
        "https://example.com"
    );
    assert_eq!(
        urls.repository.as_ref().expect("repository").as_str(),
        "https://example.com/repo.git"
    );
    assert_eq!(
        urls.documentation.as_ref().expect("documentation").as_str(),
        "https://example.com/docs"
    );
}

/// "The group does not give this key" is what a marker pointing at it
/// reports, so it has to be distinguishable from a default.
#[test]
fn a_group_key_that_is_absent_is_none() {
    let table = table("members = [\"a\"]\n");
    let root = &table.default_group;
    assert!(root.publisher.is_none());
    assert!(root.version.is_none());
    assert!(root.license.is_none());
    assert!(root.topics.is_none());
    assert!(root.metamodel.is_none());
    assert!(root.maintainers.is_none());
    assert!(root.urls.is_none());
    assert_eq!(root.inline_usages, [] as [crate::manifest::usage::Usage; 0]);
}

#[test]
fn an_unparseable_version_is_positioned_at_the_key() {
    let report = rejected("version = \"1.0\"\n");
    assert_eq!(
        texts(&report),
        [
            "`[workspace]` sets `version = \"1.0\"`, which is not a valid version: \
             unexpected end of input while parsing minor version number"
        ]
    );
    assert_eq!(places(&report), [(2, 1)]);
}

#[test]
fn an_unparseable_license_is_positioned_at_the_key() {
    let report = rejected("license = \"MIT OR\"\n");
    assert!(
        texts(&report)[0]
            .starts_with("`[workspace]` sets `license = \"MIT OR\"`, which is not a valid SPDX"),
        "unexpected message: {:?}",
        texts(&report)
    );
    assert_eq!(places(&report), [(2, 1)]);
}

/// model-toml.md calls `urls` a "table of URLs" and gives it no
/// warning clause, unlike `metamodel`: a value that is not one has
/// nowhere to go, since `website` reaches a built KPAR as an IRI.
#[test]
fn an_unparseable_url_is_an_error() {
    let report = rejected("urls.website = \"not an iri\"\n");
    assert!(
        texts(&report)[0].starts_with(
            "`[workspace.urls]` sets `website = \"not an iri\"`, which is not a valid"
        ),
        "unexpected message: {:?}",
        texts(&report)
    );
}

/// model-toml.md "Metamodel": "A value matching none of the four shapes
/// is an error naming the key and listing them." A group's value is no
/// exception: it exists to be inherited by a `[project]`, which could use
/// no other.
#[test]
fn a_metamodel_that_is_not_one_of_the_four_is_refused() {
    for value in [
        "https://example.com/spec/SysML/20250201",
        "urn:example:sysml",
        "https://www.omg.org/spec/SysML/nineteen",
        "not a url at all",
    ] {
        let report = rejected(&format!("metamodel = \"{value}\"\n"));
        assert_eq!(
            texts(&report),
            [format!(
                "`metamodel` in `[workspace]` is `{value}`, which is not one of the four values \
                 a `model.toml` recognizes;\nthey are {}",
                KnownMetamodel::RECOGNIZED
            )
            .as_str()],
            "{value}"
        );
        assert_eq!(places(&report), [(2, 1)], "{value}");
    }
}

/// A group reads its `metamodel` through the same code, and names
/// itself.
#[test]
fn a_group_metamodel_that_is_not_one_of_the_four_is_refused() {
    let report = rejected("[workspace.group.kerml]\nmetamodel = \"not a url at all\"\n");
    assert!(
        texts(&report)[0].starts_with(
            "`metamodel` in `[workspace.group.kerml]` is `not a url at all`, which is not one \
             of the four values"
        ),
        "unexpected message: {:?}",
        texts(&report)
    );
}

#[test]
fn a_maintainer_that_is_not_name_email_is_positioned_at_its_entry() {
    let report = rejected("maintainers = [\"Fine Person <fine@example.com>\", \"nobody\"]\n");
    assert_eq!(texts(&report).len(), 1, "{:?}", texts(&report));
    assert!(
        texts(&report)[0].contains("nobody"),
        "unexpected message: {:?}",
        texts(&report)
    );
    // The second entry, not the key.
    assert_eq!(places(&report), [(2, 50)]);
}

/// workspaces.md "Unknown keys": the warning holds "for the `urls`
/// tables" too, in a named group as much as in the default one.
#[test]
fn unknown_keys_in_urls_warn() {
    let validated = accepted(concat!(
        "[workspace.urls]\n",
        "favicon = \"https://example.com/f.ico\"\n",
        "\n",
        "[workspace.group.legacy.urls]\n",
        "mirror = \"https://legacy.example.com\"\n",
    ));
    let (_, warnings) = validated.into_parts();
    assert_eq!(
        messages(&warnings),
        [
            "unknown key `favicon` in `[workspace.urls]`",
            "unknown key `mirror` in `[workspace.group.legacy.urls]`",
        ]
    );
}

#[test]
fn every_problem_of_a_group_is_reported_at_once() {
    let report = rejected(concat!(
        "version = \"1.0\"\n",
        "license = \"MIT OR\"\n",
        "colour = \"red\"\n",
    ));
    assert_eq!(texts(&report).len(), 3, "{:?}", texts(&report));
    assert_eq!(places(&report), [(2, 1), (3, 1), (4, 1)]);
    assert_eq!(texts(&report)[2], "unknown key `colour` in `[workspace]`");
}

// ----------------------------------------------------------------------
// `members` and `exclude`.

#[test]
fn member_paths_are_taken_as_written() {
    let table = table("members = [\"projects/a\", \"projects/b/c\"]\n");
    let paths: Vec<&str> = table
        .members
        .iter()
        .map(|member| member.path.as_str())
        .collect();
    assert_eq!(paths, ["projects/a", "projects/b/c"]);
    // The spelling is the path: `--member <path>` matches it literally.
    let raw: Vec<&str> = table
        .members
        .iter()
        .map(|member| member.raw.as_str())
        .collect();
    assert_eq!(raw, paths);
}

/// workspaces.md "Included projects": an entry is admitted only in its
/// normalized form, rather than normalized into it.
#[test]
fn a_member_path_not_in_normal_form_is_refused() {
    let report = rejected(
        "members = [\"./projects/a\", \"projects/b/\", \"projects//c\", \"projects/./d\", \
         \"projects/x/../e\"]\n",
    );
    let form = |path: &str| {
        format!(
            "`members` path `{path}` is not in the form `members` takes: paths are relative to \
             the workspace root, `/`-separated,\nand carry no `.` or `..` component and no \
             repeated, leading or trailing slash"
        )
    };
    assert_eq!(
        texts(&report),
        [
            form("./projects/a"),
            form("projects/b/"),
            form("projects//c"),
            form("projects/./d"),
            form("projects/x/../e"),
        ]
    );
}

/// workspaces.md: "any `members` entry that resolves to the workspace
/// root itself (`"."`, `""`, `"./"`, `"folder1/.."`, ...) is rejected".
#[test]
fn a_member_path_that_is_the_root_is_refused() {
    let report = rejected("members = [\".\", \"\", \"./\", \"folder1/..\"]\n");
    assert_eq!(
        texts(&report),
        [
            "`members` path `.` is the workspace root, which must not contain a project",
            "`members` path `` is the workspace root, which must not contain a project",
            "`members` path `./` is the workspace root, which must not contain a project",
            "`members` path `folder1/..` is the workspace root, which must not contain a project",
        ]
    );
    // Each at its own entry.
    assert_eq!(places(&report).len(), 4);
    assert!(is_ascending(&places(&report)));
}

#[test]
fn a_member_path_above_the_root_is_refused() {
    let report = rejected("members = [\"../sibling\"]\n");
    assert_eq!(
        texts(&report),
        ["`members` path `../sibling` escapes the workspace root"]
    );
}

#[test]
fn a_member_path_that_is_not_relative_is_refused() {
    let report = rejected("members = [\"/abs\"]\n");
    assert_eq!(
        texts(&report),
        [
            "`members` path `/abs` is absolute; `members` and `exclude` are relative to the \
             workspace root"
        ]
    );
    let report = rejected("members = [\"a\\\\b\"]\n");
    assert!(
        texts(&report)[0].starts_with("`members` path `a\\b` contains a backslash"),
        "unexpected message: {:?}",
        texts(&report)
    );
}

/// workspaces.md: "no entry may lie inside another entry's directory:
/// nested members are forbidden for now".
#[test]
fn nested_members_are_refused() {
    for body in [
        "members = [\"projects/a\", \"projects/a/inner\"]\n",
        "members = [\"projects/a/inner\", \"projects/a\"]\n",
    ] {
        let report = rejected(body);
        assert_eq!(
            texts(&report),
            [
                "`members` path `projects/a/inner` lies inside member `projects/a`; nested \
                 members are not supported"
            ],
            "for {body:?}"
        );
    }
    // A shared prefix that is not a directory prefix is not nesting.
    let table = table("members = [\"projects/a\", \"projects/ab\"]\n");
    assert_eq!(table.members.len(), 2);
}

#[test]
fn a_member_listed_twice_is_refused() {
    let report = rejected("members = [\"projects/a\", \"projects/a\"]\n");
    assert_eq!(
        texts(&report),
        ["`members` path `projects/a` is listed twice"]
    );
}

#[test]
fn exclude_follows_the_same_per_entry_rules() {
    let report = rejected("exclude = [\"../outside\", \".\"]\n");
    assert_eq!(
        texts(&report),
        [
            "`exclude` path `../outside` escapes the workspace root",
            "`exclude` path `.` is the workspace root, which must not contain a project",
        ]
    );
}

/// workspaces.md gives `exclude` "the same path rules as `members`
/// (relative, below the root, and never the root itself)" -- that
/// parenthesis is the whole list. Two excluded directories where one
/// contains the other exclude the same tree, and so does the same
/// directory twice.
#[test]
fn exclude_accepts_nesting_and_repetition() {
    let table = table("exclude = [\"vendor\", \"vendor/inner\", \"vendor\"]\n");
    assert_eq!(table.exclude.len(), 3);
}

#[test]
fn every_bad_member_path_is_reported() {
    let report = rejected("members = [\"../a\", \"projects/b\", \"/c\"]\n");
    assert_eq!(texts(&report).len(), 2, "{:?}", texts(&report));
}

// ----------------------------------------------------------------------
// Groups, `sources`, `stdlib`, unknown keys.

#[test]
fn named_groups_are_read_under_their_names() {
    let validated = read(FULL).expect("validates");
    let table = validated.into_parts().0;
    assert_eq!(table.groups.keys().collect::<Vec<_>>(), ["legacy"]);
    let legacy = &table.groups["legacy"];
    assert_eq!(
        legacy.publisher.as_ref().expect("publisher").as_str(),
        "acme-legacy"
    );
    assert_eq!(
        legacy.version,
        Some(semver::Version::parse("0.9.0").expect("semver"))
    );
    assert_eq!(
        legacy
            .urls
            .as_ref()
            .expect("urls")
            .website
            .as_ref()
            .expect("website")
            .as_str(),
        "https://legacy.example.com"
    );

    assert!(
        table
            .group_properties(&crate::manifest::common::InheritFrom::Group(
                "legacy".to_owned()
            ))
            .is_some()
    );
    assert!(
        table
            .group_properties(&crate::manifest::common::InheritFrom::DefaultGroup)
            .is_some()
    );
    assert!(
        table
            .group_properties(&crate::manifest::common::InheritFrom::Group(
                "gone".to_owned()
            ))
            .is_none()
    );
}

/// A group whose name is not a bare TOML key could never be named by a
/// marker, so it is refused where it is defined -- once, rather than
/// once per member that tries to use it -- and its body is not read.
#[test]
fn a_group_name_that_is_not_a_bare_key_is_refused() {
    let report = rejected(concat!(
        "[workspace.group.\"legacy group\"]\n",
        "publisher = \"acme\"\n",
        "version = \"nonsense\"\n",
    ));
    assert_eq!(
        texts(&report),
        [
            "`legacy group` is not a valid workspace group name (a bare TOML key: letters, \
             digits, `-` and `_`)"
        ]
    );
}

/// workspaces.md "Common metadata": `sources` "is the key people are
/// most likely to try", so it is an error that says so rather than an
/// unknown-key warning.
#[test]
fn sources_in_the_default_group_is_an_error() {
    let report = rejected("sources = [\"src/main.sysml\"]\n");
    assert_eq!(
        texts(&report),
        ["`sources` cannot appear in `[workspace]` or in a group; it is per-project only"]
    );
    assert_eq!(places(&report), [(2, 1)]);
}

#[test]
fn sources_in_a_named_group_is_an_error() {
    let report = rejected(concat!(
        "[workspace.group.legacy]\n",
        "sources = [\"src/main.sysml\"]\n",
    ));
    assert_eq!(
        texts(&report),
        ["`sources` cannot appear in `[workspace]` or in a group; it is per-project only"]
    );
}

/// workspaces.md: `stdlib` "in `[workspace]` or a group ... is an
/// unknown key like any other (a warning)".
#[test]
fn stdlib_in_a_group_is_an_unknown_key() {
    let validated = accepted("stdlib = \"KerML\"\n");
    let (_, warnings) = validated.into_parts();
    assert_eq!(
        messages(&warnings),
        ["unknown key `stdlib` in `[workspace]`"]
    );
}

/// "`manifest-version` is not a `[project]` key at all: every file
/// carries its own", and it is not a workspace key either.
#[test]
fn manifest_version_in_a_workspace_is_an_unknown_key() {
    let validated = accepted("manifest-version = \"1.0.0\"\n");
    let (_, warnings) = validated.into_parts();
    assert_eq!(
        messages(&warnings),
        ["unknown key `manifest-version` in `[workspace]`"]
    );
}

#[test]
fn unknown_keys_of_the_workspace_table_warn() {
    let validated = accepted(concat!(
        "members = [\"a\"]\n",
        "exclude = [\"b\"]\n",
        "name = \"not a workspace key\"\n",
        "root = \"neither\"\n",
        "\n",
        "[workspace.group.legacy]\n",
        "description = \"nor this\"\n",
    ));
    let (_, warnings) = validated.into_parts();
    assert_eq!(
        messages(&warnings),
        [
            "unknown key `name` in `[workspace]`",
            "unknown key `root` in `[workspace]`",
            "unknown key `description` in `[workspace.group.legacy]`",
        ]
    );
}

#[test]
fn a_group_that_is_not_a_table_is_one_diagnostic() {
    let report = rejected("group = { legacy = 3 }\n");
    assert_eq!(
        texts(&report),
        ["`legacy` in `[workspace.group]` must be a table, but it is an integer"]
    );
}

/// Nothing in workspaces.md requires `members`, and a workspace whose
/// members have yet to be listed must not fail to read.
#[test]
fn a_workspace_with_no_members_is_legal() {
    let table = table("publisher = \"acme\"\n");
    assert_eq!(
        table.members,
        [] as [crate::manifest::workspace_table::MemberPath; 0]
    );
    assert_eq!(
        table.exclude,
        [] as [crate::manifest::workspace_table::MemberPath; 0]
    );
}

// ----------------------------------------------------------------------
// Group usages, and the table as a whole.

#[test]
fn a_default_group_usage_is_declared() {
    let validated = read(FULL).expect("validates");
    let table = validated.into_parts().0;
    let usages = &table.default_group.inline_usages;
    assert_eq!(usages.len(), 1);
    assert!(
        matches!(&usages[0], Usage::Index { name, publisher: _, version_constraint: _ } if name.as_str() == "shared-dep"),
        "unexpected usage: {:?}",
        usages[0]
    );
}

/// workspaces.md "Common usages": a group entry's `dir` "is relative to
/// ... the workspace root", so it is kept exactly as written.
#[test]
fn a_named_group_usage_is_declared() {
    let validated = read(FULL).expect("validates");
    let table = validated.into_parts().0;
    let usages = &table.groups["legacy"].inline_usages;
    assert_eq!(usages.len(), 1);
    assert!(
        matches!(
            &usages[0],
            Usage::SourcePath { name, dir, publisher: _ }
                if name.as_str() == "legacy-dep" && dir.as_str() == "legacy/dep"
        ),
        "unexpected usage: {:?}",
        usages[0]
    );
}

/// model-toml.md "Normalization": duplicates are counted per array, and
/// "The arrays are counted separately, since a workspace config holds
/// several"; workspaces.md: "Across groups the same identity is
/// expected: that is what groups are for."
#[test]
fn duplicate_usages_are_refused_per_array() {
    let report = rejected(concat!(
        "[[workspace.inline-usages]]\n",
        "publisher = \"Acme Corp\"\n",
        "name = \"lib\"\n",
        "version = \"^1\"\n",
        "\n",
        "[[workspace.inline-usages]]\n",
        "publisher = \"acme-corp\"\n",
        "name = \"lib\"\n",
        "version = \"^2\"\n",
    ));
    assert_eq!(texts(&report).len(), 1, "{:?}", texts(&report));
    assert!(
        texts(&report)[0].contains("name the same project"),
        "unexpected message: {:?}",
        texts(&report)
    );

    // The same identity in two different groups is what groups are for.
    let table = table(concat!(
        "[[workspace.inline-usages]]\n",
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "version = \"^1\"\n",
        "\n",
        "[[workspace.group.legacy.inline-usages]]\n",
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "version = \"^2\"\n",
    ));
    assert_eq!(table.default_group.inline_usages.len(), 1);
    assert_eq!(table.groups["legacy"].inline_usages.len(), 1);
}

#[test]
fn a_group_usage_that_carries_a_marker_is_refused() {
    let report = rejected(concat!(
        "[[workspace.group.legacy.inline-usages]]\n",
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "workspace = true\n",
    ));
    assert_eq!(
        texts(&report),
        [
            "usage `acme/lib` sets a `workspace` marker, which only a project's \
             `inline-usages` may do"
        ]
    );
}

#[test]
fn a_workspace_with_several_problems_reports_them_in_document_order() {
    let report = rejected(concat!(
        "members = [\"../outside\"]\n",
        "colour = \"red\"\n",
        "sources = []\n",
        "\n",
        "[workspace.group.\"bad name\"]\n",
        "publisher = \"acme\"\n",
    ));
    assert_eq!(
        texts(&report),
        [
            "`members` path `../outside` escapes the workspace root",
            "unknown key `colour` in `[workspace]`",
            "`sources` cannot appear in `[workspace]` or in a group; it is per-project only",
            "`bad name` is not a valid workspace group name (a bare TOML key: letters, \
             digits, `-` and `_`)",
        ]
    );
    let places = places(&report);
    assert!(is_ascending(&places), "not in document order: {places:?}");
}
