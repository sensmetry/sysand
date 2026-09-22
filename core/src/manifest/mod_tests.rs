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
manifest-version = \"1.0.0\"

[project]
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
";

/// The smallest file that defines a workspace.
const WORKSPACE: &str = "\
manifest-version = \"1.0.0\"

[workspace]
members = [\"projects/a\"]
";

/// `PROJECT` without its `manifest-version` line, for the tests that
/// supply their own. The line's length is not a constant worth
/// retyping: getting it wrong truncates the fixture silently.
fn project_body() -> &'static str {
    PROJECT.split_once('\n').expect("a first line").1
}

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
        "`model.toml` has no usable `manifest-version`; every `model.toml` starts with \
         `manifest-version = \"1.0.0\"`"
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
    assert!(texts(&report)[0].contains("has no usable `manifest-version`"));
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
    let report = rejected(&format!("manifest-version = \"2.0.0\"\n{}", project_body()));
    assert_eq!(
        texts(&report),
        [
            "`model.toml` is written for manifest-version 2.0.0; this Sysand supports up to \
             1.0.0; update Sysand"
        ]
    );
}

/// model-toml.md: "Additions to the common schema [...] bump the minor
/// version, and are refused by an older reader like any other version
/// it does not know: a reader that ignored them would misread the file,
/// which is the hole the key exists to close." The whole point of the
/// key being a semver version rather than a number.
#[test]
fn an_unknown_minor_is_refused() {
    let report = rejected(&format!("manifest-version = \"1.1.0\"\n{}", project_body()));
    assert_eq!(
        texts(&report),
        [
            "`model.toml` is written for manifest-version 1.1.0; this Sysand supports up to \
             1.0.0; update Sysand"
        ]
    );
}

/// "It is read before anything else, so that a file this Sysand cannot
/// read is refused with the right message rather than with a schema
/// error": a file written for a newer version may use keys that mean
/// something else here.
#[test]
fn a_newer_version_stops_the_read() {
    let report = rejected("manifest-version = \"2.0.0\"\n\n[project]\nsources = []\n");
    assert_eq!(texts(&report).len(), 1, "unexpected report: {report}");
}

#[test]
fn the_supported_version_is_accepted() {
    assert!(accepted(PROJECT).value.as_project().is_some());
}

/// model-toml.md names exactly two failures, absent and too new, so a
/// version below this one is read as this one. Almost certainly a typo;
/// refusing it would be a third rule the specification does not state.
/// Pinned so that changing it is deliberate.
#[test]
fn a_version_below_the_supported_one_is_accepted() {
    for version in ["1.0.0", "0.9.0", "0.0.0"] {
        let text = format!("manifest-version = \"{version}\"\n{}", project_body());
        assert!(parse(&text).is_ok(), "{version}");
    }
}

/// One text for absent, not a string and not a semver version, with
/// the value as the file spells it.
#[test]
fn a_manifest_version_that_is_not_a_string_is_refused() {
    // The comment moves the key off the first line: an absent version
    // is reported at the start of the file, a written one at its key,
    // and a fixture starting with the key cannot tell the two apart.
    let report = rejected(&format!(
        "# a file\nmanifest-version = 1\n{}",
        project_body()
    ));
    assert_eq!(
        texts(&report),
        [
            "`model.toml` has no usable `manifest-version` (`1`); every `model.toml` starts \
             with `manifest-version = \"1.0.0\"`"
        ]
    );
    assert_eq!(places(&report), [(2, 1)]);
}

#[test]
fn a_manifest_version_that_is_not_semver_is_refused() {
    for version in ["1", "1.0", "one"] {
        let report = rejected(&format!(
            "manifest-version = \"{version}\"\n{}",
            project_body()
        ));
        assert_eq!(
            texts(&report),
            [format!(
                "`model.toml` has no usable `manifest-version` (`\"{version}\"`); every \
                 `model.toml` starts with `manifest-version = \"1.0.0\"`"
            )],
            "{version}"
        );
    }
}

/// workspaces.md "Common metadata": "`manifest-version` is not a
/// `[project]` key at all: every file carries its own."
#[test]
fn a_manifest_version_cannot_be_inherited() {
    let report = rejected(&format!(
        "manifest-version.workspace = true\n{}",
        project_body()
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
/// with `manifest-version`."
#[test]
fn a_workspace_config_carries_a_version_too() {
    let report = rejected("[workspace]\nmembers = []\n");
    assert!(
        texts(&report)[0].contains("has no usable `manifest-version`"),
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
    let report = rejected("manifest-version = \"1.0.0\"\n[projct]\nname = \"typo\"\n");
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
    let report = rejected("manifest-version = \"1.0.0\"\nproject = 3\n");
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
    let report =
        rejected("manifest-version = \"1.0.0\"\nproject = 3\n\n[workspace]\nmembers = []\n");
    assert_eq!(texts(&report).len(), 1, "unexpected report: {report}");
    assert!(texts(&report)[0].contains("defines both"));
}

#[test]
fn a_workspace_that_is_not_a_table_is_one_diagnostic() {
    let report = rejected("manifest-version = \"1.0.0\"\nworkspace = 3\n");
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
        rejected("manifest-version = \"1.0.0\"\ninline-usages = []\n\n[workspace]\nmembers = []\n");
    assert_eq!(texts(&report).len(), 1, "unexpected report: {report}");
    assert!(texts(&report)[0].contains("no top-level `inline-usages`"));
}

/// workspaces.md "Unknown keys": a warning, in both kinds of file. Rule
/// (a)'s metamodel suffix does not apply here -- the metamodel is a
/// `[project]` key, and the top level of a file has none.
#[test]
fn an_unknown_top_level_key_is_a_warning() {
    let validated = accepted(&format!(
        "manifest-version = \"1.0.0\"\ncolour = \"red\"\n{}metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
        project_body()
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
        accepted("manifest-version = \"1.0.0\"\ncolour = \"red\"\n\n[workspace]\nmembers = []\n");
    assert_eq!(
        messages(&validated.warnings),
        ["unknown top-level key `colour`"]
    );
}

/// The exclusivity check runs before either table is read, so a file
/// with both is refused once rather than described twice.
#[test]
fn both_tables_are_refused_before_either_is_read() {
    let report = rejected(
        "manifest-version = \"1.0.0\"\n\n[project]\nversion = 3\n\n[workspace]\nmembers = 3\n",
    );
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
manifest-version = \"1.0.0\"
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
         model.toml:11:1: usage `acme/lib` names no `dir`, `kpar-path` or `kpar-url`, so it is an \
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
        ModelToml::peek(FILE, "manifest-version = \"1.0.0\"\n").expect("parses"),
        None
    );
}

// ----------------------------------------------------------------------
// The examples the specification gives
//
// Every example manifest in `model-toml.md` and `workspaces.md` parses. Both
// are in `design/`, copied byte-for-byte from where they are
// maintained; the blocks are repeated here with the section each came
// from named in a comment, because a test cannot read a fenced block
// out of a document and still say which one it meant.
//
// Most are fragments rather than whole files. A fragment is completed
// with the smallest frame that makes it one, and there are two frames:
// a project fragment gets `manifest-version` and the three required
// `[project]` keys, a workspace fragment gets `manifest-version` alone --
// adding a `[project]` to the latter would make a file with both tables,
// which is itself an error.

/// The frame a project fragment is completed with.
fn as_project_file(fragment: &str) -> String {
    format!(
        "manifest-version = \"1.0.0\"\n\n\
         [project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\n\
         sources = []\n\n{fragment}"
    )
}

/// The frame a workspace fragment is completed with.
fn as_workspace_file(fragment: &str) -> String {
    format!("manifest-version = \"1.0.0\"\n\n{fragment}")
}

/// model-toml.md, "Manifest version" and "Standard library".
#[test]
fn every_example_in_model_toml_parses() {
    // design: model-toml.md #1
    // "Manifest version". A `[project]` with only a comment in it, so
    // the required keys complete it rather than frame it.
    accepted(
        "manifest-version = \"1.0.0\"\n\n\
         [project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\n",
    );

    // design: model-toml.md #2
    // "Standard library": a publisher-less `kpar-url` naming an OMG
    // IRI. "This is the only form."
    accepted(&as_project_file(
        "[[inline-usages]]\n\
         name = \"Kernel Semantic Library\"\n\
         kpar-url = \"https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar\"\n",
    ));

    // design: model-toml.md #3
    // "Developing the standard library": the `[project]` of one of the
    // ten projects, which inherits its metamodel from a group.
    accepted(
        "manifest-version = \"1.0.0\"\n\n\
         [project]\n\
         publisher = \"OMG\"\n\
         name = \"Kernel Semantic Library\"\n\
         version = \"1.0.0\"\n\
         stdlib = \"Semantic-Library\"\n\
         metamodel.workspace = \"kerml\"\n\
         sources = [\"Base.kerml\", \"Links.kerml\", \"Occurrences.kerml\", \"Objects.kerml\"]\n",
    );
}

/// syside-metamodel.md, "`root`" and "Reading and validating". Its
/// examples are about resolution as much as parsing -- the first must
/// resolve with no warning, the second against a group, the third with
/// exactly the foreign-key warning -- so unlike the two tests above this
/// one resolves what it parses.
#[test]
fn every_example_in_syside_metamodel_md_parses_and_resolves() {
    // design: syside-metamodel.md #1
    // "`root`": a Syside project naming one of its sources as its root.
    let project = accepted(
        "manifest-version = \"1.0.0\"\n\n\
         [project]\n\
         publisher = \"acme\"\n\
         name = \"widgets\"\n\
         version = \"1.0.0\"\n\
         sources = [\"Widgets.sysml\", \"Parts.sysml\", \"Ports.sysml\"]\n\
         metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\n\
         root = \"Widgets.sysml\"\n",
    );
    assert!(
        project.warnings.is_empty(),
        "{:?}",
        messages(&project.warnings)
    );
    let resolved = project
        .value
        .into_project("model.toml")
        .expect("a project")
        .resolve(None)
        .expect("a well-formed Syside project resolves");
    assert!(
        resolved.warnings.is_empty(),
        "{:?}",
        messages(&resolved.warnings)
    );
    assert_eq!(
        resolved.value.root.as_ref().map(|root| root.as_str()),
        Some("Widgets.sysml")
    );

    // design: syside-metamodel.md #2
    // "Reading and validating": a member inheriting a Syside metamodel
    // from a group, with its own `root`. The group is not in the
    // document; it is what the group `syside` supplying the three
    // inherited keys looks like.
    let group = accepted(&as_workspace_file(
        "[workspace]\nmembers = [\"ports\"]\n\n\
         [workspace.group.syside]\npublisher = \"acme\"\nversion = \"1.0.0\"\n\
         metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\n",
    ))
    .value
    .into_workspace("model.toml")
    .expect("a workspace");
    let member = accepted(
        "manifest-version = \"1.0.0\"\n\n\
         [project]\n\
         name = \"ports\"\n\
         publisher.workspace = \"syside\"\n\
         version.workspace = \"syside\"\n\
         sources = [\"Ports.sysml\"]\n\
         metamodel.workspace = \"syside\"\n\
         root = \"Ports.sysml\"\n",
    )
    .value
    .into_project("model.toml")
    .expect("a project")
    .resolve(Some(&group))
    .expect("the member resolves against its group");
    assert!(
        member.warnings.is_empty(),
        "{:?}",
        messages(&member.warnings)
    );
    assert_eq!(
        member.value.root.as_ref().map(|root| root.as_str()),
        Some("Ports.sysml")
    );

    // design: syside-metamodel.md #3
    // "Reading and validating": an OMG project carrying `root` is a
    // valid OMG project with one warning, and no `root`.
    let legacy = accepted(
        "manifest-version = \"1.0.0\"\n\n\
         [project]\n\
         publisher = \"acme\"\n\
         name = \"legacy\"\n\
         version = \"1.0.0\"\n\
         sources = [\"Legacy.sysml\"]\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\
         root = \"Legacy.sysml\"\n",
    )
    .value
    .into_project("model.toml")
    .expect("a project")
    .resolve(None)
    .expect("an OMG project with `root` resolves");
    assert_eq!(
        messages(&legacy.warnings),
        ["`root` in `[project]` is only supported by the Syside metamodel and is ignored here"]
    );
    assert!(legacy.value.root.is_none());
}

/// Not an example in either document, and that is the point: an index
/// usage published by `OMG` is an *ordinary* usage, not a
/// standard-library one. It parses, and nothing about it is special.
///
/// It lives outside the two fidelity tests because those assert that
/// the documents' examples parse, and this is not one of them.
#[test]
fn an_index_usage_published_by_omg_is_an_ordinary_usage() {
    accepted(&as_project_file(
        "[[inline-usages]]\n\
         publisher = \"OMG\"\n\
         name = \"Kernel Semantic Library\"\n\
         version = \"^1.0.0\"\n",
    ));
    accepted(&as_workspace_file(
        "[workspace.group.kerml]\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n\n\
         [[workspace.group.kerml.inline-usages]]\n\
         publisher = \"OMG\"\nname = \"Kernel Semantic Library\"\nversion = \"^1.0.0\"\n",
    ));
}

/// workspaces.md, "Included projects" and "Common usages".
#[test]
fn every_example_in_workspaces_md_parses() {
    // design: workspaces.md #1
    // "Included projects": the one example in either document that is a
    // whole file.
    let read = accepted(
        "manifest-version = \"1.0.0\"\n\n\
         [workspace]\n\
         members = [\"folder1\", \"folder2/subfolder1\"]\n\
         exclude = [\"vendor\"]\n",
    )
    .into_parts()
    .0;
    let workspace = read.as_workspace().expect("a workspace");
    assert_eq!(workspace.members.len(), 2);
    assert_eq!(workspace.exclude.len(), 1);

    // design: workspaces.md #2
    // "Common usages": a member opting in to two group entries and
    // declaring one of its own.
    accepted(&as_project_file(
        "[[inline-usages]]\npublisher = \"acme\"\nname = \"foo\"\nworkspace = true\n\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"bar\"\nworkspace = \"legacy\"\n\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"own\"\ndir = \"../own\"\n",
    ));

    // design: workspaces.md #3
    // "Common usages": one group per language, each carrying the
    // `metamodel` of that language's release, and each carrying its
    // standard-library usage as a publisher-less `kpar-url`.
    let read = accepted(&as_workspace_file(
        "[workspace.group.kerml]\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n\n\
         [[workspace.group.kerml.inline-usages]]\n\
         name = \"Kernel Semantic Library\"\n\
         kpar-url = \"https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar\"\n\n\
         [workspace.group.sysml]\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\n\
         [[workspace.group.sysml.inline-usages]]\n\
         name = \"SysML Systems Library\"\n\
         kpar-url = \"https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar\"\n",
    ))
    .into_parts()
    .0;
    let workspace = read.as_workspace().expect("a workspace");
    assert_eq!(workspace.groups.len(), 2);
    assert_eq!(workspace.groups["kerml"].inline_usages.len(), 1);
    assert!(
        workspace.groups["kerml"].inline_usages[0].is_stdlib(),
        "{:?}",
        workspace.groups["kerml"].inline_usages[0]
    );

    accepted(&as_project_file(
        "[[inline-usages]]\nname = \"Kernel Semantic Library\"\nworkspace = \"kerml\"\n",
    ));
}

/// workspaces.md "Explicit inheritance" gives one block that is *not* a
/// manifest: every line but the first is marked `# invalid`, illustrating
/// that "inheritance cannot overlap".
///
/// It makes two different points, so it is checked as two. The first two
/// lines are valid TOML and are exactly the overlap Sysand refuses. The
/// block as a whole is not valid TOML at all -- `urls.website.workspace`
/// is assigned twice -- so a reader never reaches Sysand's rule; the
/// document is showing what not to write, not a file to parse.
#[test]
fn the_overlap_counter_example_is_refused() {
    // design: workspaces.md #4
    let report = rejected(&as_project_file(
        "urls.workspace = true\nurls.website.workspace = true # invalid\n",
    ));
    assert_eq!(
        texts(&report),
        [
            "inheritance cannot overlap: `urls` in `[project]` carries a `workspace` marker \
             and also sets `website`;\n\
             a key is either inherited or written out in full"
        ]
    );

    // design: workspaces.md #4
    let report = rejected(&as_project_file(
        "urls.workspace = true\n\
         urls.website.workspace = true # invalid\n\
         urls.website.workspace = \"some-group\" # invalid\n\
         urls.website = \"https://example.com\" # invalid\n",
    ));
    assert!(
        texts(&report)[0].starts_with("`model.toml` is not valid TOML"),
        "unexpected report: {report}"
    );
}
