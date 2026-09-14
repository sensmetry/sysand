// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::manifest::diagnostics::{Diagnostic, Report, Severity, Validated};
use crate::manifest::{ModelToml, ModelTomlKind};

const FILE: &str = "model.toml";

fn parse(text: &str) -> Result<Validated<ModelToml>, Report> {
    ModelToml::parse(FILE, text)
}

fn accepted(text: &str) -> Validated<ModelToml> {
    parse(text).expect("the manifest parses")
}

fn rejected(text: &str) -> Report {
    parse(text).expect_err("the manifest is rejected")
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

/// The smallest file that defines a project.
const PROJECT: &str = "\
manifest-version = 1

[project]
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
";

/// The smallest file that defines a workspace.
const WORKSPACE: &str = "\
manifest-version = 1

[workspace]
members = [\"projects/a\"]
";

// ----------------------------------------------------------------------
// Manifest version

/// model-toml.md: "The key is mandatory".
#[test]
fn a_manifest_version_is_required() {
    let report = rejected(
        "[project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\n",
    );
    assert_eq!(
        texts(&report)[0],
        "`model.toml` has no `manifest-version`; every `model.toml` starts with \
         `manifest-version = 1`"
    );
    // A problem with the file as a whole has no key to point at, so it
    // points at the start of the file -- and therefore sorts first.
    assert_eq!(places(&report)[0], (1, 1));
}

/// An absent version means no other schema is in play, so the rest of
/// the file is judged as usual and one read names every problem.
#[test]
fn a_missing_version_does_not_stop_the_read() {
    let report = rejected("[project]\nsources = []\n");
    assert_eq!(texts(&report).len(), 4, "unexpected report: {report}");
    assert!(texts(&report)[0].contains("has no `manifest-version`"));
    assert!(
        texts(&report)[1..]
            .iter()
            .all(|text| text.starts_with("missing key")),
        "unexpected report: {report}"
    );
}

/// The message is a text the specification fixes.
#[test]
fn a_newer_manifest_version_is_refused() {
    let report = rejected(&format!("manifest-version = 2\n{}", &PROJECT[21..]));
    assert_eq!(
        texts(&report),
        [
            "`model.toml` is written for manifest-version 2; this Sysand supports up to 1; \
          update Sysand"
        ]
    );
}

/// "It is read before anything else, so that a file this Sysand cannot
/// read is refused with the right message rather than with a schema
/// error": a file written for a newer version may use keys that mean
/// something else here.
#[test]
fn a_newer_version_stops_the_read() {
    let report = rejected("manifest-version = 2\n\n[project]\nsources = []\n");
    assert_eq!(texts(&report).len(), 1, "unexpected report: {report}");
}

#[test]
fn the_supported_version_is_accepted() {
    assert!(accepted(PROJECT).value.as_project().is_some());
}

/// model-toml.md names exactly two failures, absent and too new, so a
/// smaller number is read as this version. Almost certainly a typo for
/// `1`; refusing it would be a third rule the specification does not
/// state. Pinned so that changing it is deliberate.
#[test]
fn a_version_of_zero_or_less_is_accepted() {
    for version in ["0", "-1"] {
        let text = format!("manifest-version = {version}\n{}", &PROJECT[21..]);
        assert!(parse(&text).is_ok(), "{version}");
    }
}

#[test]
fn a_manifest_version_that_is_not_an_integer_is_refused() {
    let report = rejected(&format!("manifest-version = \"1\"\n{}", &PROJECT[21..]));
    assert_eq!(
        texts(&report),
        ["`manifest-version` in the top level must be an integer, but it is a string"]
    );
}

/// workspaces.md "Common metadata": "`manifest-version` is not a
/// `[project]` key at all: every file carries its own."
#[test]
fn a_manifest_version_cannot_be_inherited() {
    let report = rejected(&format!(
        "manifest-version.workspace = true\n{}",
        &PROJECT[21..]
    ));
    assert_eq!(
        texts(&report),
        [
            "`manifest-version` in the top level cannot be inherited from the workspace;\n\
             every file carries its own"
        ]
    );
}

/// workspaces.md: "Like every `model.toml`, a workspace config starts
/// with `manifest-version = 1`."
#[test]
fn a_workspace_config_carries_a_version_too() {
    let report = rejected("[workspace]\nmembers = []\n");
    assert!(
        texts(&report)[0].contains("has no `manifest-version`"),
        "unexpected report: {report}"
    );
}

// ----------------------------------------------------------------------
// Which table the file defines

#[test]
fn a_project_file_reads_as_a_project() {
    let read = accepted(PROJECT).into_parts().0;
    assert_eq!(
        read.as_project().expect("a project").project.name.as_str(),
        "widgets"
    );
    assert!(read.as_workspace().is_none());
}

#[test]
fn a_workspace_file_reads_as_a_workspace() {
    let read = accepted(WORKSPACE).into_parts().0;
    assert_eq!(read.as_workspace().expect("a workspace").members.len(), 1);
    assert!(read.as_project().is_none());
}

/// A file that defines both tables is an error.
#[test]
fn both_tables_in_one_file_are_refused() {
    let report = rejected(&format!("{PROJECT}\n[workspace]\nmembers = []\n"));
    assert_eq!(
        texts(&report),
        ["`model.toml` defines both `[project]` and `[workspace]`;\n\
             a workspace root must not contain a project"]
    );
}

/// model-toml.md: "a `model.toml` with
/// neither is an error rather than a file that defines nothing -- which
/// is what a mistyped table name would otherwise become".
#[test]
fn neither_table_is_refused() {
    let report = rejected("manifest-version = 1\n[projct]\nname = \"typo\"\n");
    assert_eq!(
        texts(&report)[0],
        "`model.toml` defines neither `[project]` nor `[workspace]`;\n\
         a `model.toml` defines one or the other"
    );
    assert_eq!(places(&report)[0], (1, 1));
}

/// Which table a file defines is decided on whether the key is written,
/// not on whether it reads as a table: `project = 3` writes `project`,
/// and calling that file "one that defines neither" would be both false
/// and further from the fix.
#[test]
fn a_scalar_project_key_still_defines_a_project() {
    let report = rejected("manifest-version = 1\nproject = 3\n");
    assert_eq!(
        texts(&report),
        ["`project` in the top level must be a table, but it is an integer"]
    );
}

/// What that costs: a stray scalar `project` beside a real `[workspace]`
/// is refused as the exclusivity error, and its type is never mentioned.
/// The fix -- delete the stray key -- is the same either way, and "a
/// workspace root must not contain a project" is what says so.
#[test]
fn a_scalar_project_key_beside_a_workspace_is_the_exclusivity_error() {
    let report = rejected("manifest-version = 1\nproject = 3\n\n[workspace]\nmembers = []\n");
    assert_eq!(texts(&report).len(), 1, "unexpected report: {report}");
    assert!(texts(&report)[0].contains("defines both"));
}

#[test]
fn a_workspace_that_is_not_a_table_is_one_diagnostic() {
    let report = rejected("manifest-version = 1\nworkspace = 3\n");
    assert_eq!(
        texts(&report),
        ["`workspace` in the top level must be a table, but it is an integer"]
    );
}

#[test]
fn a_workspace_config_has_no_top_level_inline_usages() {
    let report = rejected(&format!(
        "{WORKSPACE}\n[[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\nversion = \"^1\"\n"
    ));
    assert_eq!(
        texts(&report),
        [
            "a workspace config has no top-level `inline-usages`; declare common usages \
             under `[workspace]`"
        ]
    );
    // Unlike the file-level diagnostics, this one has a key to point at.
    assert_eq!(places(&report), [(6, 3)]);
}

/// "one at the top level of a workspace config is an error" is about the
/// key being there, so an explicit empty array is refused too.
#[test]
fn an_empty_inline_usages_array_is_refused_too() {
    let report =
        rejected("manifest-version = 1\ninline-usages = []\n\n[workspace]\nmembers = []\n");
    assert_eq!(texts(&report).len(), 1, "unexpected report: {report}");
    assert!(texts(&report)[0].contains("no top-level `inline-usages`"));
}

/// workspaces.md "Unknown keys": a warning, in both kinds of file. Rule
/// (a)'s metamodel suffix does not apply here -- the metamodel is a
/// `[project]` key, and the top level of a file has none.
#[test]
fn an_unknown_top_level_key_is_a_warning() {
    let validated = accepted(&format!(
        "manifest-version = 1\ncolour = \"red\"\n{}metamodel = \"https://example.com/other\"\n",
        &PROJECT[21..]
    ));
    assert_eq!(
        messages(&validated.warnings)[0],
        "unknown top-level key `colour`"
    );
    assert!(
        validated
            .warnings
            .iter()
            .all(|warning| warning.severity == Severity::Warning)
    );

    let validated =
        accepted("manifest-version = 1\ncolour = \"red\"\n\n[workspace]\nmembers = []\n");
    assert_eq!(
        messages(&validated.warnings),
        ["unknown top-level key `colour`"]
    );
}

/// The exclusivity check runs before either table is read, so a file
/// with both is refused once rather than described twice.
#[test]
fn both_tables_are_refused_before_either_is_read() {
    let report =
        rejected("manifest-version = 1\n\n[project]\nversion = 3\n\n[workspace]\nmembers = 3\n");
    assert_eq!(texts(&report).len(), 1, "unexpected report: {report}");
}

// ----------------------------------------------------------------------
// Composition

/// The golden report: errors and warnings in one list, in the order the
/// file reads, each positioned. Everything it asserts is asserted
/// piecewise elsewhere; what it pins is the composition.
#[test]
fn a_report_covers_both_levels_of_the_file() {
    const TEXT: &str = "\
manifest-version = 1
colour = \"red\"

[project]
publisher = \"acme\"
name = \"widgets\"
version = 3
shape = \"round\"
sources = []

[[inline-usages]]
publisher = \"acme\"
name = \"lib\"
";
    let report = rejected(TEXT);
    assert_eq!(
        report.to_string(),
        "model.toml:2:1: unknown top-level key `colour`\n\
         model.toml:7:1: `version` in `[project]` must be a string, but it is an integer\n\
         model.toml:8:1: unknown key `shape` in `[project]`\n\
         model.toml:11:1: usage `acme/lib` names no `dir`, `kpar-path` or `url`, so it is an \
         index usage; an index usage requires a `version` constraint"
    );
}

/// A file that is not TOML at all is the one failure that precedes every
/// rule of the manifest.
#[test]
fn a_syntax_error_is_still_one_diagnostic() {
    let report = rejected("manifest-version =\n");
    assert_eq!(texts(&report).len(), 1);
    assert!(
        texts(&report)[0].starts_with("`model.toml` is not valid TOML"),
        "unexpected report: {report}"
    );
}

/// A caller that opened a file expecting one of the two says so once,
/// here, rather than in each of the two places that do it.
#[test]
fn the_wrong_variant_says_so() {
    let workspace = accepted(WORKSPACE).into_parts().0;
    assert_eq!(
        workspace
            .into_project(FILE)
            .expect_err("a workspace")
            .message,
        "`model.toml` defines a workspace, not a project"
    );
    let project = accepted(PROJECT).into_parts().0;
    assert_eq!(
        project.into_workspace(FILE).expect_err("a project").message,
        "`model.toml` defines a project, not a workspace"
    );
}

/// workspaces.md's upward walk asks one question of every ancestor, and
/// must not be stopped by an unrelated file's problems.
#[test]
fn peek_answers_without_validating() {
    assert_eq!(
        ModelToml::peek(FILE, "[workspace]\nmembers = 3\n").expect("parses"),
        Some(ModelTomlKind::Workspace)
    );
    assert_eq!(
        ModelToml::peek(FILE, "[project]\n").expect("parses"),
        Some(ModelTomlKind::Project)
    );
    // Not TOML at all is still an answer a walk has to hear.
    ModelToml::peek(FILE, "x =\n").expect_err("not TOML");
}

#[test]
fn peek_on_a_file_with_neither_table_is_none() {
    assert_eq!(
        ModelToml::peek(FILE, "manifest-version = 1\n").expect("parses"),
        None
    );
}
