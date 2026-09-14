// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::manifest::common::InheritFrom;
use crate::manifest::diagnostics::{Report, Severity};
use crate::manifest::spans::Positions;
use crate::manifest::spans::Source;
use crate::manifest::usage::{
    MemberUsage, Usage, UsageRaw, UsageSite, check_duplicates, normalize_local_path,
};
use crate::model::{IndexName, IndexPublisher};

/// The reading flow of a `model.toml`, minus the `[project]` and
/// `[workspace]` tables: parse once, harvest the spans, read the entries
/// out of the values.
fn read(text: &str) -> (Vec<UsageRaw>, Positions<'_>) {
    let (table, positions) = Source::new("model.toml", text)
        .read()
        .expect("the test document parses");
    let raws = entries(&table, "inline-usages");
    (raws, positions)
}

/// The `[[inline-usages]]` of a parsed document, wherever they sit.
fn entries(table: &toml::Table, key: &str) -> Vec<UsageRaw> {
    table
        .get(key)
        .and_then(toml::Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(toml::Value::as_table)
                .map(UsageRaw::from_table)
                .collect()
        })
        .unwrap_or_default()
}

fn site(index: usize) -> UsageSite<'static> {
    UsageSite {
        array: "inline-usages",
        index,
    }
}

fn members(text: &str) -> Vec<Result<MemberUsage, Report>> {
    let (raws, positions) = read(text);
    raws.into_iter()
        .enumerate()
        .map(|(index, raw)| raw.validate_member(&site(index), &positions))
        .collect()
}

/// One `[[inline-usages]]` entry, whose keys therefore start on line 2.
fn one(body: &str) -> String {
    format!("[[inline-usages]]\n{body}")
}

fn member(body: &str) -> MemberUsage {
    members(&one(body))
        .pop()
        .expect("one entry")
        .expect("the entry validates")
}

fn rejected(body: &str) -> Report {
    members(&one(body))
        .pop()
        .expect("one entry")
        .expect_err("the entry is rejected")
}

fn texts(report: &Report) -> Vec<String> {
    report
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
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

fn publisher(raw: &str) -> IndexPublisher {
    IndexPublisher::parse(raw).expect("a valid publisher")
}

fn name(raw: &str) -> IndexName {
    IndexName::parse(raw).expect("a valid name")
}

// ----------------------------------------------------------------------
// The raw shape.

/// model-toml.md "Usages" fixes the key set; `git`/`rev`/`tag`/`branch`
/// are recognized so that declaring one can be refused as the
/// postponement it is, rather than as an unknown key.
#[test]
fn a_usage_entry_deserializes_every_recognized_key() {
    let (raws, _) = read(&one(concat!(
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "version = \"^1.2\"\n",
        "dir = \"../lib\"\n",
        "kpar-path = \"../lib.kpar\"\n",
        "url = \"https://example.com/lib.kpar\"\n",
        "git = \"https://example.com/lib.git\"\n",
        "rev = \"abc123\"\n",
        "tag = \"v1\"\n",
        "branch = \"main\"\n",
        "workspace = true\n",
    )));
    let raw = &raws[0];
    assert_eq!(raw.publisher.as_deref(), Some("acme"));
    assert_eq!(raw.name.as_deref(), Some("lib"));
    assert_eq!(raw.version.as_deref(), Some("^1.2"));
    assert_eq!(raw.dir.as_deref(), Some("../lib"));
    assert_eq!(raw.kpar_path.as_deref(), Some("../lib.kpar"));
    assert_eq!(raw.url.as_deref(), Some("https://example.com/lib.kpar"));
    assert_eq!(raw.git.as_deref(), Some("https://example.com/lib.git"));
    assert_eq!(raw.rev.as_deref(), Some("abc123"));
    assert_eq!(raw.tag.as_deref(), Some("v1"));
    assert_eq!(raw.branch.as_deref(), Some("main"));
    assert!(raw.workspace.is_some());
    assert!(raw.unknown.is_empty());
}

/// model-toml.md "Reading and validating": parsing never fails on
/// content, so an unknown key is collected and a value of the wrong
/// shape is carried to validation rather than stopping the read.
#[test]
fn unaccepted_keys_are_collected_not_dropped() {
    let (raws, _) = read(&one(concat!(
        "publisher = \"a !b\"\n",
        "name = \"lib\"\n",
        "colour = \"red\"\n",
        "shape = \"round\"\n",
    )));
    let raw = &raws[0];
    assert_eq!(raw.unknown.iter().collect::<Vec<_>>(), ["colour", "shape"]);
    assert_eq!(raw.publisher.as_deref(), Some("a !b"));
}

/// A value of the wrong TOML *type* is a diagnostic like any other, so
/// it accumulates with what else the entry gets wrong rather than
/// stopping the read of the rest of the file.
#[test]
fn a_toml_type_error_accumulates_like_every_other() {
    let report = rejected("name = 3\npublisher = \"a !b\"\n");
    assert_eq!(
        texts(&report),
        [
            "`name` in usage `a !b/<none>` must be a string, but it is an integer",
            "publisher must be 3-50 ASCII alphanumeric chars, with single ` ` or `-` separators between words",
        ]
    );
    assert_eq!(places(&report), [(2, 1), (3, 1)]);
}

#[test]
fn a_usage_raw_round_trips_through_toml() {
    for body in [
        "publisher = \"acme\"\nname = \"lib\"\nversion = \"^1.2\"\n",
        "publisher = \"acme\"\nname = \"lib\"\ndir = \"../lib\"\n",
        "publisher = \"acme\"\nname = \"lib\"\nkpar-path = \"../lib.kpar\"\n",
        "publisher = \"acme\"\nname = \"lib\"\nurl = \"https://example.com/l.kpar\"\n",
        "publisher = \"acme\"\nname = \"lib\"\nworkspace = \"legacy\"\n",
    ] {
        let (raws, _) = read(&one(body));
        let written = toml_edit::ser::to_string(&raws[0]).expect("serializes");
        let (again, _) = read(&one(&written));
        assert_eq!(raws[0], again[0], "round trip of {body:?}");
    }
}

// ----------------------------------------------------------------------
// Values and kind detection.

/// model-toml.md "Usages": "an index usage is the entry that carries
/// none" of the kind keys.
#[test]
fn a_usage_without_a_kind_key_is_an_index_usage() {
    assert_eq!(
        member("publisher = \"acme\"\nname = \"lib\"\nversion = \"^1.2\"\n"),
        MemberUsage::Declared(Usage::Index {
            publisher: publisher("acme"),
            name: name("lib"),
            version_constraint: semver::VersionReq::parse("^1.2").expect("a requirement"),
        })
    );
}

/// A value of the right type but the wrong shape is Sysand's to report,
/// with a position, and it must not stop the entries after it from being
/// read.
#[test]
fn an_invalid_publisher_is_a_positioned_diagnostic() {
    let text = concat!(
        "[[inline-usages]]\n",
        "publisher = \"a !b\"\n",
        "name = \"lib\"\n",
        "version = \"^1\"\n",
        "\n",
        "[[inline-usages]]\n",
        "publisher = \"acme\"\n",
        "name = \"other\"\n",
        "version = \"^1\"\n",
    );
    let results = members(text);
    let report = results[0].as_ref().expect_err("rejected");
    assert_eq!(
        texts(report),
        [
            "publisher must be 3-50 ASCII alphanumeric chars, with single ` ` or `-` separators between words"
        ]
    );
    assert_eq!(places(report), [(2, 1)]);
    assert!(results[1].is_ok(), "the entry after it still validates");
}

#[test]
fn an_invalid_version_requirement_is_reported() {
    let report = rejected("publisher = \"acme\"\nname = \"lib\"\nversion = \"wat\"\n");
    assert_eq!(
        texts(&report),
        ["usage `acme/lib` sets `version = \"wat\"`, which is not a version requirement"]
    );
    assert_eq!(places(&report), [(4, 1)]);
}

#[test]
fn an_invalid_url_is_reported() {
    let report = rejected("publisher = \"acme\"\nname = \"lib\"\nurl = \"not an iri\"\n");
    assert_eq!(
        texts(&report),
        ["usage `acme/lib` sets `url = \"not an iri\"`, which is not a valid IRI"]
    );
    assert_eq!(places(&report), [(4, 1)]);
}

/// model-toml.md gives a version requirement only to the index kind;
/// "remote-kpar: url, publisher, name -- ... version constraint not
/// supported".
/// model-toml.md "Usages": a local path is taken exactly as written, so
/// only the normal form is admitted -- `..` may lead, since such a path
/// may leave the directory it is relative to, and an absolute path keeps
/// its root.
#[test]
fn a_local_path_in_normal_form_is_accepted() {
    for path in [
        "lib",
        "libs/lib",
        "..",
        "../lib",
        "../../libs/lib",
        "/opt/libs/lib",
    ] {
        let MemberUsage::Declared(usage) = member(&format!(
            "publisher = \"acme\"\nname = \"lib\"\ndir = \"{path}\"\n"
        )) else {
            panic!("`{path}` is a declared usage");
        };
        assert_eq!(
            usage.local_path().map(|p| p.as_str()),
            Some(path),
            "`{path}` is taken as written"
        );
    }
}

#[test]
fn a_local_path_not_in_normal_form_is_refused() {
    for (key, path) in [
        ("dir", ""),
        ("dir", "."),
        ("dir", "./lib"),
        ("dir", "lib/"),
        ("dir", "libs//lib"),
        ("dir", "libs/./lib"),
        ("dir", "libs/../lib"),
        ("dir", "../libs/../lib"),
        ("dir", "/opt/../lib"),
        ("dir", "libs\\\\lib"),
        ("kpar-path", "./lib.kpar"),
    ] {
        let report = rejected(&format!(
            "publisher = \"acme\"\nname = \"lib\"\n{key} = \"{path}\"\n"
        ));
        let written = path.replace("\\\\", "\\");
        assert_eq!(
            texts(&report),
            [format!(
                "usage `acme/lib` sets `{key} = \"{written}\"`, which is not in the form a local \
                 path takes:\n`/`-separated, with `..` only as leading components, no `.` \
                 component, no repeated or trailing slash and no backslash"
            )],
            "`{path}`"
        );
        assert_eq!(places(&report), [(4, 1)], "`{path}`");
    }
}

/// What a command writes when it converts a path it was given.
#[test]
fn a_local_path_is_normalized_into_the_form_a_manifest_admits() {
    for (raw, normal) in [
        ("lib", Some("lib")),
        ("./lib/", Some("lib")),
        ("libs//lib", Some("libs/lib")),
        ("libs/./x/../lib", Some("libs/lib")),
        ("../lib", Some("../lib")),
        ("./../../lib", Some("../../lib")),
        ("../a/../../lib", Some("../../lib")),
        ("lib/..", None),
        (".", None),
    ] {
        assert_eq!(
            normalize_local_path(raw).as_ref().map(|p| p.as_str()),
            normal,
            "`{raw}`"
        );
    }
}

#[test]
fn only_index_usages_take_a_version_constraint() {
    for kind in [
        "dir = \"../lib\"",
        "kpar-path = \"../lib.kpar\"",
        "url = \"https://example.com/lib.kpar\"",
    ] {
        let report = rejected(&format!(
            "publisher = \"acme\"\nname = \"lib\"\nversion = \"^1\"\n{kind}\n"
        ));
        assert_eq!(
            texts(&report),
            ["usage `acme/lib` sets `version`, which only an index usage takes"],
            "for {kind}"
        );
        assert_eq!(places(&report), [(4, 1)], "for {kind}");
    }
}

/// model-toml.md: "index: publisher, name, version constraint". An entry
/// with no kind key is an index usage, so a missing `version` is a
/// missing constraint rather than a missing kind.
#[test]
fn an_index_usage_requires_a_version() {
    let report = rejected("publisher = \"acme\"\nname = \"lib\"\n");
    assert_eq!(
        texts(&report),
        [
            "usage `acme/lib` names no `dir`, `kpar-path` or `url`, so it is an index usage; \
             an index usage requires a `version` constraint"
        ]
    );
    assert_eq!(places(&report), [(1, 1)]);
}

/// model-toml.md: of the kind keys, "at most one may be
/// present".
#[test]
fn a_usage_carrying_two_kind_keys_is_rejected() {
    for kinds in [
        "dir = \"../lib\"\nurl = \"https://example.com/l.kpar\"",
        "dir = \"../lib\"\nkpar-path = \"../lib.kpar\"",
        "dir = \"../lib\"\nkpar-path = \"../lib.kpar\"\nurl = \"https://example.com/l.kpar\"",
    ] {
        let report = rejected(&format!("publisher = \"acme\"\nname = \"lib\"\n{kinds}\n"));
        assert_eq!(
            texts(&report),
            ["usage `acme/lib` carries more than one of `dir`, `kpar-path`, `url`"],
            "for {kinds}"
        );
        assert_eq!(places(&report), [(1, 1)], "for {kinds}");
    }
}

/// model-toml.md: "Every usage carries `publisher` and
/// `name`: that pair is the identity a usage is resolved by".
#[test]
fn every_usage_names_its_publisher() {
    let report = rejected("name = \"lib\"\nversion = \"^1\"\n");
    assert_eq!(
        texts(&report),
        ["usage `lib` has no `publisher`; every usage names its publisher"]
    );
    assert_eq!(places(&report), [(1, 1)]);
}

/// With no name there is nothing to call the entry, so
/// the publisher is reported as a plain missing key too.
#[test]
fn a_usage_without_a_name_reports_a_missing_key() {
    let report = rejected("publisher = \"acme\"\nversion = \"^1\"\n");
    assert_eq!(
        texts(&report),
        ["missing key `name` in `[[inline-usages]]`"]
    );

    let both = rejected("version = \"^1\"\n");
    assert_eq!(
        texts(&both),
        [
            "missing key `name` in `[[inline-usages]]`",
            "missing key `publisher` in `[[inline-usages]]`",
        ]
    );
}

/// model-toml.md names `git` a kind key and marks it "Postponed to after
/// v1: not declarable in v1", which is what the reader says -- the same
/// thing `sysand migrate` says about a git resource.
#[test]
fn git_usage_is_not_declarable() {
    let report = rejected(concat!(
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "git = \"https://example.com/lib.git\"\n",
    ));
    assert_eq!(
        texts(&report),
        [
            "usage `acme/lib` declares `git`; git usages are postponed to after v1 and cannot \
             be declared in `model.toml`"
        ]
    );
    assert_eq!(places(&report), [(4, 1)]);

    // The ref keys belong to the same postponed kind, not to the unknown
    // ones: naming them as unknown would tell the reader they will never
    // exist.
    let refs = rejected(concat!(
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "version = \"^1\"\n",
        "rev = \"abc123\"\n",
        "tag = \"v1\"\n",
    ));
    assert_eq!(
        texts(&refs),
        [
            "usage `acme/lib` declares `rev`, `tag`; git usages are postponed to after v1 and \
             cannot be declared in `model.toml`"
        ]
    );
}

/// workspaces.md "Unknown keys": "The one exception is a usage entry: an
/// unknown key inside an `[[inline-usages]]` ... entry is an error",
/// because a reader "cannot tell a kind key it does not know from any
/// other unknown key".
#[test]
fn unknown_keys_in_a_usage_are_errors_not_warnings() {
    let report = rejected(concat!(
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "version = \"^1\"\n",
        "colour = \"red\"\n",
        "shape = \"round\"\n",
    ));
    assert_eq!(
        texts(&report),
        [
            "unknown key `colour` in usage `acme/lib`; a usage entry accepts only `publisher`, \
             `name`, `version`, `dir`, `kpar-path`, `url`, `workspace`",
            "unknown key `shape` in usage `acme/lib`; a usage entry accepts only `publisher`, \
             `name`, `version`, `dir`, `kpar-path`, `url`, `workspace`",
        ]
    );
    assert_eq!(places(&report), [(5, 1), (6, 1)]);
    assert!(
        report
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Error)
    );
}

/// model-toml.md "Reading and validating": Sysand's validation "reports
/// every problem it finds in one report".
#[test]
fn a_usage_reports_every_problem_at_once() {
    let report = rejected("name = \"lib\"\nversion = \"^1\"\ncolour = \"red\"\n");
    assert_eq!(
        texts(&report),
        [
            "usage `lib` has no `publisher`; every usage names its publisher",
            "unknown key `colour` in usage `<none>/lib`; a usage entry accepts only \
             `publisher`, `name`, `version`, `dir`, `kpar-path`, `url`, `workspace`",
        ]
    );
}

/// The checks run in field order and unknown keys are found
/// alphabetically; the report is put back into the order the file reads.
#[test]
fn diagnostics_come_back_in_document_order() {
    let report = rejected(concat!(
        "zebra = \"last alphabetically, first in the file\"\n",
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "version = \"^1\"\n",
        "apple = \"first alphabetically, last in the file\"\n",
    ));
    assert_eq!(places(&report), [(2, 1), (6, 1)]);
}

// ----------------------------------------------------------------------
// Member entries and opt-in markers.

#[test]
fn a_member_entry_without_a_marker_is_declared() {
    assert!(matches!(
        member("publisher = \"acme\"\nname = \"lib\"\ndir = \"../lib\"\n"),
        MemberUsage::Declared(Usage::SourcePath { .. })
    ));
}

/// workspaces.md "Common usages": "an entry that gives only the identity
/// plus a `workspace` marker takes the matching group entry whole.
/// `workspace = true` selects the default group, `workspace =
/// "group-name"` a named one".
#[test]
fn a_workspace_marker_makes_an_entry_an_opt_in() {
    // `at` is asserted on its own by
    // `an_opt_in_records_where_its_marker_is`, so the identity and the
    // group are what these two compare.
    assert!(matches!(
        member("publisher = \"acme\"\nname = \"lib\"\nworkspace = true\n"),
        MemberUsage::Inherited {
            publisher: p,
            name: n,
            from: InheritFrom::DefaultGroup,
            ..
        } if p == publisher("acme") && n == name("lib")
    ));
    assert!(matches!(
        member("publisher = \"acme\"\nname = \"lib\"\nworkspace = \"legacy\"\n"),
        MemberUsage::Inherited {
            publisher: p,
            name: n,
            from: InheritFrom::Group(group),
            ..
        } if p == publisher("acme") && n == name("lib") && group == "legacy"
    ));
}

/// The opt-in's marker carries its own position, so that "the group has
/// no entry with that publisher and name" can point at the `workspace`
/// key long after the file's text is gone.
#[test]
fn an_opt_in_records_where_its_marker_is() {
    let MemberUsage::Inherited { at, .. } =
        member("publisher = \"acme\"\nname = \"lib\"\nworkspace = true\n")
    else {
        panic!("an opt-in");
    };
    let position = at.expect("positioned");
    assert_eq!(position.file, "model.toml");
    // Line 1 is `[[inline-usages]]`, so the marker is the fourth line.
    assert_eq!((position.line, position.column), (4, 1));
}

/// workspaces.md: "The group entry is used as-is; the member cannot
/// change any of its fields."
#[test]
fn an_opt_in_may_set_only_its_identity() {
    let report = rejected(concat!(
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "workspace = true\n",
        "version = \"^1\"\n",
    ));
    assert_eq!(
        texts(&report),
        [
            "usage `acme/lib` inherits from the default group, so it may set only `publisher`, \
             `name` and `workspace`"
        ]
    );
    assert_eq!(places(&report), [(5, 1)]);
}

/// There is no "inherit nothing" marker: a key that is not inherited
/// carries its own value.
#[test]
fn workspace_false_is_not_a_marker() {
    let report = rejected("publisher = \"acme\"\nname = \"lib\"\nworkspace = false\n");
    assert_eq!(
        texts(&report),
        [
            "`workspace = false` is not allowed; a key that is not inherited carries its own \
             value"
        ]
    );
    assert_eq!(places(&report), [(4, 1)]);
}

#[test]
fn a_marker_naming_an_impossible_group_is_rejected() {
    let report = rejected("publisher = \"acme\"\nname = \"lib\"\nworkspace = \"bad name!\"\n");
    assert_eq!(
        texts(&report),
        [
            "`bad name!` is not a valid workspace group name (a bare TOML key: letters, digits, \
             `-` and `_`)"
        ]
    );
}

/// A workspace group's own entries have nothing above them to inherit
/// from.
#[test]
fn a_group_entry_cannot_carry_a_marker() {
    let text =
        "[[workspace.inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\nworkspace = true\n";
    let (table, positions) = Source::new("model.toml", text).read().expect("parses");
    let group = table["workspace"].as_table().expect("a table");
    let raws = entries(group, "inline-usages");
    let report = raws[0]
        .clone()
        .validate_declared(
            &UsageSite {
                array: "workspace.inline-usages",
                index: 0,
            },
            &positions,
        )
        .expect_err("rejected");
    assert_eq!(
        texts(&report),
        [
            "usage `acme/lib` sets a `workspace` marker, which only a project's `inline-usages` \
             may do"
        ]
    );
}

// ----------------------------------------------------------------------
// Duplicates.

fn duplicates(text: &str) -> Vec<crate::manifest::diagnostics::Diagnostic> {
    let (raws, positions) = read(text);
    let entries: Vec<(UsageSite<'_>, MemberUsage)> = raws
        .into_iter()
        .enumerate()
        .filter_map(|(index, raw)| {
            let site = site(index);
            raw.validate_member(&site, &positions)
                .ok()
                .map(|usage| (site, usage))
        })
        .collect();
    check_duplicates(&entries, &positions)
}

/// model-toml.md "Normalization": "two entries whose identities normalize
/// to the same value are the same project declared twice and are
/// rejected, whether or not the two entries agree on kind or
/// constraint".
#[test]
fn usages_that_normalize_to_the_same_identity_are_duplicates() {
    let found = duplicates(concat!(
        "[[inline-usages]]\n",
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "version = \"^1\"\n",
        "\n",
        "[[inline-usages]]\n",
        "publisher = \"ACME\"\n",
        "name = \"Lib\"\n",
        "dir = \"../lib\"\n",
    ));
    assert_eq!(
        found
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>(),
        [
            "usages `acme/lib` and `ACME/Lib` name the same project; two entries whose \
             publisher and name normalize to the same value are a duplicate"
        ]
    );
    // The second entry is the one the reader would delete.
    let position = found[0].position.as_ref().expect("positioned");
    assert_eq!((position.line, position.column), (6, 1));
}

/// An entry that failed validation is not in the list, so a duplicate
/// found after it must not be located by its place in that list.
#[test]
fn duplicate_positions_survive_a_rejected_entry() {
    let found = duplicates(concat!(
        "[[inline-usages]]\n",
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "version = \"^1\"\n",
        "\n",
        "[[inline-usages]]\n",
        "name = \"broken\"\n",
        "\n",
        "[[inline-usages]]\n",
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "dir = \"../lib\"\n",
    ));
    let position = found[0].position.as_ref().expect("positioned");
    assert_eq!((position.line, position.column), (9, 1));
}

#[test]
fn distinct_identities_are_not_duplicates() {
    assert_eq!(
        duplicates(concat!(
            "[[inline-usages]]\n",
            "publisher = \"acme\"\n",
            "name = \"lib\"\n",
            "version = \"^1\"\n",
            "\n",
            "[[inline-usages]]\n",
            "publisher = \"acme\"\n",
            "name = \"other\"\n",
            "dir = \"../other\"\n",
        )),
        [] as [crate::manifest::diagnostics::Diagnostic; 0]
    );
    // Every colliding pair is reported, so one edit round fixes the file.
    assert_eq!(
        duplicates(concat!(
            "[[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\nversion = \"^1\"\n",
            "[[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\nversion = \"^2\"\n",
            "[[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\nversion = \"^3\"\n",
        ))
        .len(),
        2
    );
}

// ----------------------------------------------------------------------
// Writing a usage back out.

#[test]
fn every_usage_kind_round_trips_through_its_raw_form() {
    for body in [
        "publisher = \"acme\"\nname = \"lib\"\nversion = \"^1.2\"\n",
        "publisher = \"acme\"\nname = \"lib\"\ndir = \"../lib\"\n",
        "publisher = \"acme\"\nname = \"lib\"\nkpar-path = \"../lib.kpar\"\n",
        "publisher = \"acme\"\nname = \"lib\"\nurl = \"https://example.com/l.kpar\"\n",
    ] {
        let MemberUsage::Declared(usage) = member(body) else {
            panic!("{body:?} declares a usage");
        };
        let raw = UsageRaw::from(usage.clone());
        let text = one(&toml_edit::ser::to_string(&raw).expect("serializes"));
        let again = members(&text)
            .pop()
            .expect("one entry")
            .expect("the written entry validates");
        assert_eq!(
            again,
            MemberUsage::Declared(usage),
            "round trip of {body:?}"
        );
    }
}

#[test]
fn an_opt_in_round_trips_as_a_marker() {
    let usage = member("publisher = \"acme\"\nname = \"lib\"\nworkspace = \"legacy\"\n");
    let raw = UsageRaw::from(usage.clone());
    let written = toml_edit::ser::to_string(&raw).expect("serializes");
    assert_eq!(
        written,
        "publisher = \"acme\"\nname = \"lib\"\nworkspace = \"legacy\"\n"
    );
    let again = members(&one(&written))
        .pop()
        .expect("one entry")
        .expect("validates");
    assert_eq!(again, usage);
}

// ----------------------------------------------------------------------
// An entry is read from a plain table, so that no value
// can stop the read of the file it is in.

#[test]
fn a_usage_entry_reads_from_a_table() {
    let (raws, _) = read(&one(concat!(
        "publisher = \"acme\"\n",
        "name = \"lib\"\n",
        "version = \"^1\"\n",
        "dir = \"../lib\"\n",
        "kpar-path = \"../lib.kpar\"\n",
        "url = \"https://example.com/l.kpar\"\n",
        "git = \"https://example.com/l.git\"\n",
        "rev = \"abc\"\n",
        "tag = \"v1\"\n",
        "branch = \"main\"\n",
        "workspace = \"legacy\"\n",
    )));
    let raw = &raws[0];
    assert_eq!(raw.publisher.as_deref(), Some("acme"));
    assert_eq!(raw.name.as_deref(), Some("lib"));
    assert_eq!(raw.version.as_deref(), Some("^1"));
    assert_eq!(raw.dir.as_deref(), Some("../lib"));
    assert_eq!(raw.kpar_path.as_deref(), Some("../lib.kpar"));
    assert_eq!(raw.url.as_deref(), Some("https://example.com/l.kpar"));
    assert_eq!(raw.git.as_deref(), Some("https://example.com/l.git"));
    assert_eq!(raw.rev.as_deref(), Some("abc"));
    assert_eq!(raw.tag.as_deref(), Some("v1"));
    assert_eq!(raw.branch.as_deref(), Some("main"));
    assert_eq!(
        raw.workspace,
        Some(crate::manifest::common::InheritMarker::Group(
            "legacy".to_owned()
        ))
    );
    assert!(raw.wrong_typed.is_empty());
}

/// A key whose value has the wrong type leaves nothing behind to read,
/// so the entry has no shape: asking it for a kind would invent one.
/// Here `dir` is gone, and without this rule the entry would *also* be
/// reported as an index usage that is missing its `version`.
#[test]
fn a_wrong_typed_usage_key_does_not_make_up_a_kind() {
    let report = rejected("publisher = \"acme\"\nname = \"lib\"\ndir = 3\n");
    assert_eq!(
        texts(&report),
        ["`dir` in usage `acme/lib` must be a string, but it is an integer"]
    );
    assert_eq!(places(&report), [(4, 1)]);
}

/// The marker is the one key of an entry that is neither a string nor a
/// table: `workspace = true` and `workspace = "group"` are both it.
#[test]
fn a_wrong_typed_marker_says_what_it_should_be() {
    let report = rejected("publisher = \"acme\"\nname = \"lib\"\nworkspace = 3\n");
    assert_eq!(
        texts(&report),
        ["`workspace` in usage `acme/lib` must be a boolean or a string, but it is an integer"]
    );
}

/// A wrong-typed identity key is one problem, not two: it must not also
/// be reported as a key that is missing.
#[test]
fn a_wrong_typed_name_is_not_also_a_missing_name() {
    let report = rejected("publisher = \"acme\"\nname = 3\n");
    assert_eq!(
        texts(&report),
        ["`name` in usage `acme/<none>` must be a string, but it is an integer"]
    );
}
