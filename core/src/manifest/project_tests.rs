// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::manifest::common::{InheritFrom, Sources};
use crate::manifest::diagnostics::{Diagnostic, Report, Severity, Validated};
use crate::manifest::inherit::MetaProperty;
use crate::manifest::project::{ProjectManifest, ProjectTable, ResolvedProject};
use crate::manifest::spans::Source;
use crate::manifest::usage::{MemberUsage, Usage, UsageBase};
use crate::manifest::workspace_table::WorkspaceTable;
use crate::model::{IndexName, IndexPublisher, KnownMetamodel, MetamodelFamily, SpecKind};
use fluent_uri::Iri;

/// The project side of a document, read the way `ModelToml::parse` will
/// read it.
fn read(text: &str) -> Result<Validated<ProjectManifest>, Report> {
    let (table, positions) = Source::new("model.toml", text)
        .read()
        .expect("the test document parses");
    ProjectManifest::validate(&table, &positions)
}

/// A `[project]` whose body is `body`, so that its keys start on line 2.
fn one(body: &str) -> String {
    format!("[project]\n{body}")
}

fn accepted(body: &str) -> Validated<ProjectManifest> {
    read(&one(body)).expect("the manifest validates")
}

fn manifest(body: &str) -> ProjectManifest {
    accepted(body).into_parts().0
}

fn rejected(body: &str) -> Report {
    read(&one(body)).expect_err("the manifest is rejected")
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

/// Whether every diagnostic sits strictly after the one before it.
fn is_ascending(places: &[(usize, usize)]) -> bool {
    places.windows(2).all(|pair| match pair {
        [earlier, later] => earlier < later,
        _ => true,
    })
}

/// The three keys `[project]` requires, so that a test about one other
/// key does not also report these.
const REQUIRED: &str = "\
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
";

/// [`REQUIRED`] plus the two keys a project needs to *resolve*: the
/// `metamodel` every `model.toml` declares, and the `sources` an
/// OMG-metamodel project lists. For the tests that are about something
/// else.
const RESOLVABLE: &str = "\
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
";

/// [`REQUIRED`] plus `sources`, without `key`: a key cannot be written
/// twice in one table, so a test about how `key` is read has to leave it
/// out of the boilerplate.
fn without(key: &str) -> String {
    let mut body: Vec<&str> = vec![
        "publisher = \"acme\"",
        "name = \"widgets\"",
        "version = \"1.2.3\"",
        "sources = []",
    ];
    body.retain(|line| !line.starts_with(&format!("{key} ")));
    format!("{}\n", body.join("\n"))
}

/// Every key model-toml.md "Main project metadata" documents, plus one
/// `[[inline-usages]]` entry per kind declarable in v1. Ported from the
/// prototype's `FULL_PROJECT_TOML`.
const FULL: &str = "\
manifest-version = \"1.0.0\"

[project]
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = [\"src/widgets.sysml\", \"src/parts.kerml\"]
license = \"MIT OR Apache-2.0\"
description = \"Example widgets project\"
topics = [\"examples\", \"widgets\"]
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
maintainers = [\"Jane Doe <jane@example.com>\", \"John Roe <john@example.com>\"]

[project.urls]
website = \"https://example.com\"
repository = \"https://example.com/repo.git\"
documentation = \"https://example.com/docs\"

[[inline-usages]]
publisher = \"acme\"
name = \"index-dep\"
version = \"^1.0\"

[[inline-usages]]
publisher = \"acme\"
name = \"src-dep\"
dir = \"deps/src-dep\"

[[inline-usages]]
publisher = \"acme\"
name = \"kpar-dep\"
kpar-path = \"deps/kpar-dep.kpar\"

[[inline-usages]]
publisher = \"acme\"
name = \"remote-kpar-dep\"
kpar-url = \"https://example.com/remote.kpar\"
";

fn provided<T>(property: &MetaProperty<T>) -> &T {
    property.provided().expect("a value, not a marker")
}

/// Whether a key carries a marker pointing at the default group.
fn default<T>(property: &MetaProperty<T>) -> bool {
    property.inherited().expect("a marker").from == InheritFrom::DefaultGroup
}

#[test]
fn every_documented_key_is_read() {
    let validated = read(FULL).expect("the manifest validates");
    assert_eq!(
        validated.warnings,
        [] as [crate::manifest::diagnostics::Diagnostic; 0]
    );
    let read = validated.into_parts().0;
    let project = &read.project;

    assert_eq!(project.name.as_str(), "widgets");
    assert_eq!(provided(&project.publisher).as_str(), "acme");
    assert_eq!(provided(&project.version).to_string(), "1.2.3");
    assert_eq!(
        project
            .sources
            .as_ref()
            .expect("sources")
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>(),
        ["src/widgets.sysml", "src/parts.kerml"]
    );
    assert_eq!(
        provided(project.license.as_ref().expect("license")).to_string(),
        "MIT OR Apache-2.0"
    );
    assert_eq!(
        project.description.as_deref(),
        Some("Example widgets project")
    );
    assert_eq!(
        provided(project.topics.as_ref().expect("topics")),
        &["examples".to_owned(), "widgets".to_owned()]
    );
    assert_eq!(
        provided(project.metamodel.as_ref().expect("metamodel")).to_string(),
        "https://www.omg.org/spec/SysML/20250201"
    );
    assert_eq!(
        provided(project.maintainers.as_ref().expect("maintainers"))
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [
            "Jane Doe <jane@example.com>".to_owned(),
            "John Roe <john@example.com>".to_owned()
        ]
    );
    assert!(project.stdlib.is_none());
    assert!(project.root.is_none());

    let urls = provided(project.urls.as_ref().expect("urls"));
    assert_eq!(
        provided(urls.website.as_ref().expect("website")).as_str(),
        "https://example.com"
    );
    assert_eq!(
        provided(urls.documentation.as_ref().expect("documentation")).as_str(),
        "https://example.com/docs"
    );
    assert_eq!(
        provided(urls.repository.as_ref().expect("repository")).as_str(),
        "https://example.com/repo.git"
    );

    assert_eq!(read.inline_usages.len(), 4);
    assert!(matches!(
        &read.inline_usages[0],
        MemberUsage::Declared(Usage::Index { name, version_constraint, publisher: _ })
            if name.as_str() == "index-dep" && version_constraint.to_string() == "^1.0"
    ));
    assert!(matches!(
        &read.inline_usages[1],
        MemberUsage::Declared(Usage::SourcePath { name, dir, publisher: _ })
            if name.as_str() == "src-dep" && dir.as_str() == "deps/src-dep"
    ));
    assert!(matches!(
        &read.inline_usages[2],
        MemberUsage::Declared(Usage::KparPath { name, kpar_path, publisher: _ })
            if name.as_str() == "kpar-dep" && kpar_path.as_str() == "deps/kpar-dep.kpar"
    ));
    assert!(matches!(
        &read.inline_usages[3],
        MemberUsage::Declared(Usage::RemoteKpar { name, url, publisher: _ })
            if name.as_str() == "remote-kpar-dep" && url.as_str() == "https://example.com/remote.kpar"
    ));
}

/// model-toml.md "Reading and validating": every field is optional at
/// parse time, and the missing ones are reported together, by Sysand.
#[test]
fn the_three_required_keys_are_reported_together() {
    let report = rejected("sources = []\n");
    assert_eq!(
        texts(&report),
        [
            "missing key `publisher` in `[project]`",
            "missing key `name` in `[project]`",
            "missing key `version` in `[project]`",
        ]
    );
}

/// A key that is written but cannot be read has already been reported;
/// reporting it a second time as missing would be two complaints about
/// one mistake.
#[test]
fn a_required_key_that_is_present_but_unreadable_is_not_also_missing() {
    for (body, message) in [
        (
            "publisher = 3\nname = \"widgets\"\nversion = \"1.2.3\"\n",
            "`publisher` in `[project]` must be a string, but it is an integer",
        ),
        (
            "publisher = \"a !b\"\nname = \"widgets\"\nversion = \"1.2.3\"\n",
            "publisher must be 3-50 ASCII alphanumeric chars, with single ` ` or `-` separators between words",
        ),
        (
            "publisher.workspace = 3\nname = \"widgets\"\nversion = \"1.2.3\"\n",
            "`publisher.workspace` in `[project]` must be a boolean or a string, but it is \
             an integer",
        ),
    ] {
        assert_eq!(texts(&rejected(body)), [message], "{body}");
    }
}

/// Rule (a) is a json-only rule: a `model.toml` whose metamodel is
/// unrecognized is refused before any required-key check runs, so the
/// suffix has nothing to explain.
#[test]
fn a_missing_key_does_not_name_the_metamodel() {
    let report = rejected("name = \"widgets\"\nmetamodel = \"https://example.com/other\"\n");
    assert_eq!(
        texts(&report)
            .iter()
            .filter(|text| text.contains("this may be because"))
            .count(),
        0,
        "unexpected report: {report}"
    );
    assert!(
        texts(&report)
            .iter()
            .any(|text| text.contains("is not one of the four values")),
        "unexpected report: {report}"
    );
}

/// model-toml.md `sources`: entries are normalized on read, and the rules
/// of `Sources` apply. Each complaint points at the entry that caused it.
#[test]
fn sources_entries_are_checked_and_positioned() {
    for (entries, fragment, column) in [
        (
            "[\"../escape.sysml\"]",
            "is not in the form `sources` takes",
            12,
        ),
        ("[\"src/\"]", "names a directory", 12),
        ("[\"notes.md\"]", "is not a KerML", 12),
        ("[\"a.sysml\", \"a.sysml\"]", "listed more than once", 23),
    ] {
        let body = format!("{REQUIRED}sources = {entries}\n");
        let report = rejected(&body);
        assert_eq!(texts(&report).len(), 1, "{entries}");
        assert!(
            texts(&report)[0].contains(fragment),
            "{entries}: unexpected message: {}",
            texts(&report)[0]
        );
        // The duplicate points at the *second* spelling: that is the
        // entry a reader would delete.
        assert_eq!(places(&report), [(5, column)], "{entries}");
    }
}

/// model-toml.md "Reading and validating": every problem is listed at
/// once, so each bad entry is reported at itself -- and each repeat of an
/// entry is a duplicate of its own, not one complaint at the last copy.
#[test]
fn every_bad_sources_entry_is_reported_at_itself() {
    let report = rejected(&format!(
        "{REQUIRED}sources = [\"a.sysml\", \"notes.md\", \"a.sysml\", \"a.sysml\"]\n"
    ));
    assert_eq!(texts(&report).len(), 3, "unexpected report: {report}");
    assert!(texts(&report)[0].contains("`notes.md` is not a KerML"));
    assert!(texts(&report)[1].contains("`a.sysml` is listed more than once"));
    assert!(texts(&report)[2].contains("`a.sysml` is listed more than once"));
    assert_eq!(places(&report), [(5, 23), (5, 35), (5, 46)]);
}

/// model-toml.md: `sources` "cannot be inherited by a member ..., since
/// the file layout of one project says nothing about another's". The
/// half of `common_tests.rs`'s test of this name that survives the move
/// off `serde`.
#[test]
fn sources_cannot_be_inherited() {
    for marker in [
        "sources.workspace = true",
        "sources.workspace = \"legacy\"",
        "sources.workspace = false",
        "sources = { workspace = true }",
    ] {
        let report = rejected(&format!("{REQUIRED}{marker}\n"));
        assert!(
            texts(&report)
                .iter()
                .any(|text| text.contains("cannot be inherited")),
            "{marker}: unexpected report: {report}"
        );
    }
}

/// workspaces.md "Unknown keys": a warning, in `[project]` and in the
/// `urls` table alike. Rule (a)'s suffix is gone from both: a
/// `model.toml` with an unrecognized metamodel does not get this far.
#[test]
fn an_unknown_key_is_a_positioned_warning() {
    let validated = accepted(&format!("{RESOLVABLE}future-project-key = \"?\"\n"));
    assert_eq!(
        messages(&validated.warnings),
        ["unknown key `future-project-key` in `[project]`"]
    );

    let text = format!(
        "[project]\n{RESOLVABLE}future-project-key = \"?\"\n\n\
         [project.urls]\nwebsite = \"https://example.com\"\nfuture-url = \"?\"\n"
    );
    let validated = read(&text).expect("the manifest validates");
    assert_eq!(
        messages(&validated.warnings),
        [
            "unknown key `future-project-key` in `[project]`",
            "unknown key `future-url` in `[project.urls]`",
        ]
    );
    assert!(
        validated
            .warnings
            .iter()
            .all(|warning| warning.severity == Severity::Warning)
    );
}

/// model-toml.md "Metamodel": "Exactly four values
/// are recognized [...] A value matching none of the four shapes is an
/// error naming the key and listing them." A URL of another family and
/// a value that is not a URL at all are one answer: neither is one of
/// the four.
#[test]
fn an_unrecognized_metamodel_is_refused() {
    for value in [
        "https://example.com/other",
        "urn:example:sysml",
        "not a url at all",
        "https://www.omg.org/spec/SysML/2025",
    ] {
        let report = rejected(&format!(
            "{REQUIRED}sources = []\nmetamodel = \"{value}\"\n"
        ));
        assert_eq!(
            texts(&report),
            [format!(
                "`metamodel` in `[project]` is `{value}`, which is not one of the four values \
                 a `model.toml` recognizes;\nthey are `https://www.omg.org/spec/KerML/<date>`, \
                 `https://www.omg.org/spec/SysML/<date>`, \
                 `https://www.sysand.com/spec/KerML/<date>` or \
                 `https://www.sysand.com/spec/SysML/<date>`"
            )],
            "{value}"
        );
        assert_eq!(places(&report), [(6, 1)], "{value}");
    }
}

/// An unrecognized metamodel says nothing about the schema either, so
/// rule (b) does not run over `root`: one error, not an error and a
/// warning about a key whose schema the reader cannot know. The
/// companion of `a_metamodel_that_is_not_a_string_does_not_decide_the_schema`,
/// which covers the value that is not a string at all.
#[test]
fn an_unrecognized_metamodel_does_not_decide_the_schema() {
    let report = rejected(&format!(
        "{REQUIRED}sources = []\nmetamodel = \"https://example.com/other\"\nroot = \"main.sysml\"\n"
    ));
    assert_eq!(texts(&report).len(), 1, "{:?}", texts(&report));
}

// ---------------------------------------------------------------------
// `root` (design/syside-metamodel.md). The key is read as written and
// judged only once the family is known, in `resolve`.

const OMG_SYSML: &str = "https://www.omg.org/spec/SysML/20250201";
const SYSIDE_SYSML: &str = "https://www.sysand.com/spec/SysML/20250201";
const SYSIDE_KERML: &str = "https://www.sysand.com/spec/KerML/20250201";

const ROOT_MISSING: &str = "missing key `root` in `[project]`; the Syside metamodel requires it";
const ROOT_IN_OMG: &str =
    "`root` in `[project]` is only supported by the Syside metamodel and is ignored here";
const SYSIDE_SOURCES_MISSING: &str = "missing key `sources` in `[project]`; the Syside metamodel requires it\n\
                     (`root` names one of its entries)";

/// [`REQUIRED`] plus what a Syside project needs to resolve.
fn syside(sources: &str, root: &str) -> String {
    format!("{REQUIRED}metamodel = \"{SYSIDE_SYSML}\"\n{sources}{root}")
}

/// Read and resolve outside any workspace, keeping both stages'
/// warnings apart: the reader's must be empty for a Syside project, and
/// what `resolve` says is the subject.
fn standalone(body: &str) -> Result<Validated<ResolvedProject>, Report> {
    let validated = accepted(body);
    assert!(
        validated.warnings.is_empty(),
        "nothing about `root` is judged before the family is known: {:?}",
        messages(&validated.warnings)
    );
    validated.value.resolve(None)
}

/// A hand-written Syside `model.toml` reads with no warning. `root` is
/// one of the `sources`, and the resolved project carries it.
#[test]
fn a_syside_manifest_reads_with_no_warning() {
    let body = syside(
        "sources = [\"main.sysml\", \"lib.sysml\"]\n",
        "root = \"main.sysml\"\n",
    );
    let resolved = standalone(&body).expect("a well-formed Syside project resolves");
    assert!(
        resolved.warnings.is_empty(),
        "{:?}",
        messages(&resolved.warnings)
    );
    let project = resolved.value;
    assert_eq!(project.metamodel.family, MetamodelFamily::Syside);
    assert_eq!(
        project.root.as_ref().map(|root| root.as_str()),
        Some("main.sysml")
    );
    assert_eq!(
        project
            .sources
            .iter()
            .map(|entry| entry.as_str())
            .collect::<Vec<_>>(),
        ["main.sysml", "lib.sysml"]
    );
}

/// `root` is required of every Syside project. With `sources` present,
/// it is the only complaint.
#[test]
fn a_syside_project_without_root_is_missing_the_key() {
    let report = standalone(&syside("sources = [\"main.sysml\"]\n", "")).expect_err("no root");
    assert_eq!(texts(&report), [ROOT_MISSING]);
}

/// Both required keys missing: both reported, in one report, in key
/// order.
#[test]
fn root_and_sources_are_both_reported_when_both_are_missing() {
    let report = standalone(&syside("", "")).expect_err("neither key");
    assert_eq!(texts(&report), [SYSIDE_SOURCES_MISSING, ROOT_MISSING]);
}

/// `root` goes through the checks a `sources` entry goes through, with
/// its own wording. A `root` that fails them is reported once, at
/// `root`, and is not *also* reported missing.
#[test]
fn root_is_held_to_the_sources_entry_checks() {
    for (written, text) in [
        (
            "\"/abs.sysml\"",
            "`root` `/abs.sysml` is absolute; `root` is relative to the project root, like a \
             `sources` entry",
        ),
        (
            "\"a\\\\b.sysml\"",
            "`root` `a\\b.sysml` contains a backslash `\\`;\nit is `/`-separated, so that a \
             manifest reads the same way on every operating system",
        ),
        (
            "\"a\\u0000.sysml\"",
            "`root` `a\0.sysml` contains a NULL character",
        ),
        (
            "\"../up.sysml\"",
            "`root` `../up.sysml` is not in the form a `sources` entry takes: relative, \
             `/`-separated,\nwith no `.` or `..` component and no repeated or leading slash",
        ),
        (
            "\"src/\"",
            "`root` `src/` names a directory; `root` names a single model file",
        ),
        (
            "\"notes.txt\"",
            "`root` `notes.txt` is not a KerML (`.kerml`) or SysML (`.sysml`) file",
        ),
    ] {
        let body = syside(
            "sources = [\"main.sysml\"]\n",
            &format!("root = {written}\n"),
        );
        let report = standalone(&body).expect_err(written);
        assert_eq!(texts(&report), [text], "{written}");
        // Line 6 of the document: `[project]`, three required keys,
        // `metamodel`, `sources`, then `root`.
        assert_eq!(places(&report), [(7, 1)], "{written}");
    }
}

/// A `root` that is not a string is the reader's type wording, raised
/// where the family is known -- and nothing else: not missing, since
/// the key was written.
#[test]
fn a_root_that_is_not_a_string_is_a_type_error_in_a_syside_project() {
    let report =
        standalone(&syside("sources = [\"main.sysml\"]\n", "root = true\n")).expect_err("a bool");
    assert_eq!(
        texts(&report),
        ["`root` in `[project]` must be a string, but it is a boolean"]
    );
    assert_eq!(places(&report), [(7, 1)]);
}

/// `root` is a `sources` entry, and every `sources` entry is already
/// held to the metamodel, so an inadmissible `root` raises nothing of
/// its own. One file, one complaint.
#[test]
fn an_inadmissible_root_is_complained_about_once_as_a_sources_entry() {
    let body = format!(
        "{REQUIRED}metamodel = \"{SYSIDE_KERML}\"\nsources = [\"main.sysml\"]\n\
         root = \"main.sysml\"\n"
    );
    let report = standalone(&body).expect_err("a SysML file in a KerML project");
    assert_eq!(
        texts(&report),
        [format!(
            "`sources` entry `main.sysml` is a SysML file, which the metamodel `{SYSIDE_KERML}` \
             does not admit;\na KerML project lists `.kerml` files, a SysML project `.sysml` and \
             `.kerml` files"
        )]
    );
}

/// `root` names one of the `sources` entries, compared as written like
/// the entries themselves.
/// `sources = []` beside a `root` is therefore this error too.
#[test]
fn root_must_be_listed_in_sources() {
    for sources in ["sources = [\"lib.sysml\"]\n", "sources = []\n"] {
        let report =
            standalone(&syside(sources, "root = \"main.sysml\"\n")).expect_err("not listed");
        assert_eq!(
            texts(&report),
            ["`root` is `main.sysml`, which is not listed in `sources`"],
            "{sources}"
        );
        assert_eq!(places(&report), [(7, 1)], "{sources}");
    }
}

/// The membership check needs a list to be absent from: with no
/// `sources` at all the complaint is the missing key, once.
#[test]
fn root_membership_is_not_checked_when_sources_is_absent() {
    let report = standalone(&syside("", "root = \"main.sysml\"\n")).expect_err("no sources");
    assert_eq!(texts(&report), [SYSIDE_SOURCES_MISSING]);
}

/// `root` in an OMG project is a warning, raised where the family is known:
/// model-toml.md "Metamodel", rule (b) -- an OMG project with `root` gets
/// only the warning and is built from `sources`. *Whatever* the value: a
/// path, a bool, an escaping path, an integer. None of them is judged, and
/// the resolved project carries no `root`.
#[test]
fn root_in_an_omg_project_is_a_warning() {
    for root in ["\"main.sysml\"", "true", "\"../escapes.sysml\"", "3"] {
        let body = format!("{RESOLVABLE}root = {root}\n");
        let validated = accepted(&body);
        assert!(validated.warnings.is_empty(), "{root}");
        assert!(
            validated.value.project.root.is_some(),
            "{root}: kept as written"
        );
        let resolved = validated
            .value
            .resolve(None)
            .expect("`sources` is there, so `root` costs only the warning");
        assert_eq!(messages(&resolved.warnings), [ROOT_IN_OMG], "{root}");
        assert!(
            resolved.value.root.is_none(),
            "{root}: built from `sources`"
        );
    }

    // Condition 1: `root` alone does not stand in for `sources`; the
    // warning comes with the error.
    let report = manifest(&format!(
        "{REQUIRED}metamodel = \"{OMG_SYSML}\"\nroot = \"main.sysml\"\n"
    ))
    .resolve(None)
    .expect_err("no sources");
    assert_eq!(
        texts(&report),
        [
            "missing key `sources` in `[project]`; the OMG metamodel requires it\n\
             (a project with no source files writes `sources = []`)",
            ROOT_IN_OMG
        ]
    );
}

/// A Syside metamodel that arrives by marker is a Syside project; its
/// `root` is not a foreign key.
#[test]
fn root_is_not_foreign_when_the_syside_metamodel_is_inherited() {
    let group = workspace(&format!(
        "[workspace]\nmembers = []\nmetamodel = \"{SYSIDE_SYSML}\"\n"
    ));
    let body = format!(
        "{REQUIRED}metamodel.workspace = true\nsources = [\"main.sysml\"]\nroot = \"main.sysml\"\n"
    );
    let resolved = manifest(&body)
        .resolve(Some(&group))
        .expect("the member resolves");
    assert!(
        resolved.warnings.is_empty(),
        "{:?}",
        messages(&resolved.warnings)
    );
    assert_eq!(
        resolved.value.root.as_ref().map(|root| root.as_str()),
        Some("main.sysml")
    );
}

/// `root` is not inheritable, so a marker on it is refused -- and does
/// not leave a `root` behind as if one had been written.
#[test]
fn a_marker_on_root_is_refused() {
    let report = rejected(&format!("{RESOLVABLE}root.workspace = true\n"));
    assert_eq!(
        texts(&report),
        [
            "`root` in `[project]` cannot be inherited from the workspace;\n\
             it is a per-project entry point"
        ]
    );
}

/// The required keys follow the family: `sources` for both, `root` for
/// Syside as well. What a project with only the three common keys is
/// told, for each of the four values.
#[test]
fn the_required_keys_follow_the_family_for_all_four_metamodels() {
    for (url, expected) in [
        (
            "https://www.omg.org/spec/KerML/20250201",
            vec![
                "missing key `sources` in `[project]`; the OMG metamodel requires it\n\
                 (a project with no source files writes `sources = []`)",
            ],
        ),
        (
            OMG_SYSML,
            vec![
                "missing key `sources` in `[project]`; the OMG metamodel requires it\n\
                 (a project with no source files writes `sources = []`)",
            ],
        ),
        (SYSIDE_KERML, vec![SYSIDE_SOURCES_MISSING, ROOT_MISSING]),
        (SYSIDE_SYSML, vec![SYSIDE_SOURCES_MISSING, ROOT_MISSING]),
    ] {
        let report = manifest(&format!("{REQUIRED}metamodel = \"{url}\"\n"))
            .resolve(None)
            .expect_err(url);
        assert_eq!(texts(&report), expected, "{url}");
    }
}

// ---------------------------------------------------------------------

/// A `metamodel` that could not be read says nothing about the schema,
/// so neither of the judgements that depend on it runs: without this a
/// project declaring a Syside metamodel with a typo'd *type* would also
/// be told its `root` belongs to another metamodel.
#[test]
fn an_unreadable_metamodel_suppresses_the_root_warning() {
    let report = rejected(&format!(
        "{REQUIRED}sources = []\nmetamodel = 3\nroot = \"main.sysml\"\n"
    ));
    assert_eq!(
        texts(&report),
        ["`metamodel` in `[project]` must be a string, but it is an integer"]
    );
}

/// workspaces.md "Common metadata" lists the keys a member may inherit.
/// The port of the prototype's `project_inheritance_markers`.
#[test]
fn every_inheritable_key_accepts_a_marker() {
    let read = manifest(
        "name = \"member\"\n\
         publisher.workspace = true\n\
         version.workspace = true\n\
         sources = [\"src/main.sysml\"]\n\
         license.workspace = \"alt-group\"\n\
         urls.workspace = true\n\
         topics.workspace = true\n\
         metamodel.workspace = true\n\
         maintainers.workspace = \"alt-group\"\n",
    );
    let project = read.project;
    assert!(default(&project.publisher));
    assert!(default(&project.version));
    assert!(default(project.urls.as_ref().expect("urls")));
    assert!(default(project.topics.as_ref().expect("topics")));
    assert!(default(project.metamodel.as_ref().expect("metamodel")));
    for property in [
        project.license.as_ref().map(MetaProperty::inherited),
        project.maintainers.as_ref().map(MetaProperty::inherited),
    ] {
        assert_eq!(
            property.expect("present").expect("a marker").from,
            InheritFrom::Group("alt-group".to_owned())
        );
    }
}

/// workspaces.md: `urls` may be inherited "whole table or individual
/// keys".
#[test]
fn a_urls_table_inherits_whole_or_per_key() {
    let read = manifest(&format!(
        "{RESOLVABLE}\
         urls.website.workspace = \"legacy\"\n\
         urls.repository = \"https://example.com/repo.git\"\n"
    ));
    let urls = provided(read.project.urls.as_ref().expect("urls"));
    assert_eq!(
        urls.website
            .as_ref()
            .expect("website")
            .inherited()
            .expect("a marker")
            .from,
        InheritFrom::Group("legacy".to_owned())
    );
    assert_eq!(
        provided(urls.repository.as_ref().expect("repository")).as_str(),
        "https://example.com/repo.git"
    );
    assert!(urls.documentation.is_none());
}

/// workspaces.md "Common metadata": every key that is deliberately
/// outside inheritance says so in its own words.
#[test]
fn every_non_inheritable_key_refuses_a_marker() {
    for (key, reason) in [
        ("name", "it identifies one project"),
        ("description", "it identifies one project"),
        (
            "sources",
            "every member writes out its own source files in full",
        ),
        ("stdlib", "it is a per-project identifier"),
        ("root", "it is a per-project entry point"),
    ] {
        // The key under test is dropped from the required three: a
        // dotted `name.workspace` beside a `name = "..."` is a TOML
        // syntax error, not something this reader ever sees.
        let body = format!("{}{key}.workspace = true\n", without(key));
        let report = rejected(&body);
        assert!(
            texts(&report).contains(
                &format!(
                    "`{key}` in `[project]` cannot be inherited from the workspace;\n{reason}"
                )
                .as_str()
            ),
            "{key}: unexpected report: {report}"
        );
    }
}

/// model-toml.md "Standard library": `stdlib` is "validated as a single
/// path segment (`[A-Za-z0-9][A-Za-z0-9-]*`) so it cannot carry `/` or
/// `..` into an IRI".
#[test]
fn stdlib_is_read_as_a_path_segment() {
    let read = manifest(
        "publisher = \"OMG\"\nname = \"Kernel Semantic Library\"\nversion = \"1.0.0\"\n\
         sources = []\nstdlib = \"Semantic-Library\"\n",
    );
    assert_eq!(
        read.project.stdlib.as_ref().expect("stdlib").as_str(),
        "Semantic-Library"
    );
    for bad in ["", "-x", "a/b", "..", "a b", "a.kpar"] {
        let report = rejected(&format!("{RESOLVABLE}stdlib = \"{bad}\"\n"));
        assert_eq!(texts(&report).len(), 1, "{bad}");
    }
}

/// One value of the wrong TOML type does not stop the read of the rest:
/// the whole point of the raw-value layer.
#[test]
fn a_wrong_typed_key_is_one_diagnostic_and_the_read_continues() {
    let report = rejected(&format!(
        "{RESOLVABLE}description = 1\ntopics = \"a\"\nlicense = true\n"
    ));
    assert_eq!(
        texts(&report),
        [
            "`description` in `[project]` must be a string, but it is an integer",
            "`topics` in `[project]` must be an array of strings, but it is a string",
            "`license` in `[project]` must be a string, but it is a boolean",
        ]
    );
}

/// `inline-usages` is a top-level array, not a key of `[project]`.
#[test]
fn inline_usages_are_read_from_the_top_level() {
    let text = format!(
        "[project]\n{RESOLVABLE}\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"own\"\ndir = \"../own\"\n\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"shared\"\nworkspace = true\n"
    );
    let read = read(&text).expect("validates").into_parts().0;
    assert!(matches!(
        &read.inline_usages[0],
        MemberUsage::Declared(Usage::SourcePath {
            publisher: _,
            name: _,
            dir: _
        })
    ));
    assert!(matches!(
        &read.inline_usages[1],
        MemberUsage::Inherited {
            publisher: _,
            name: _,
            from: _,
            at: _
        }
    ));
    assert!(read.inherits());
}

/// workspaces.md "Common usages": "Within any one array ... two entries
/// of the same identity are rejected as duplicates ..., which includes an
/// inherited entry colliding with one the member declares itself."
#[test]
fn duplicate_usages_are_rejected_per_array() {
    let text = format!(
        "[project]\n{RESOLVABLE}\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\ndir = \"../lib\"\n\n\
         [[inline-usages]]\npublisher = \"Acme\"\nname = \"lib\"\nworkspace = true\n"
    );
    let report = read(&text).expect_err("duplicates");
    assert!(
        texts(&report)[0].contains("name the same project"),
        "unexpected report: {report}"
    );
}

/// A report with several problems reads from the top of the file down.
#[test]
fn a_report_is_in_document_order() {
    let report = rejected(&format!(
        "{REQUIRED}sources = [\"notes.md\"]\ntopics = 3\nmaintainers = [\"no email\"]\n"
    ));
    assert_eq!(texts(&report).len(), 3, "unexpected report: {report}");
    assert!(is_ascending(&places(&report)), "{:?}", places(&report));
}

// ----------------------------------------------------------------------
// Resolving against a workspace

/// Every key workspaces.md "Common metadata" lists, in the default group
/// and in a named one.
const WORKSPACE: &str = "\
[workspace]
members = [\"projects/a\"]
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

[workspace.group.legacy.urls]
website = \"https://legacy.example.com\"

[[workspace.group.legacy.inline-usages]]
publisher = \"acme\"
name = \"legacy-dep\"
dir = \"legacy/dep\"
";

fn workspace(text: &str) -> WorkspaceTable {
    let (table, positions) = Source::new("model.toml", text)
        .read()
        .expect("the workspace parses");
    let workspace = table
        .get("workspace")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();
    WorkspaceTable::validate(&workspace, &positions)
        .expect("the workspace validates")
        .into_parts()
        .0
}

/// Resolve a `[project]` body against `WORKSPACE`.
fn member(body: &str) -> Result<Validated<ResolvedProject>, Report> {
    manifest(body).resolve(Some(&workspace(WORKSPACE)))
}

fn resolved(body: &str) -> ResolvedProject {
    member(body).expect("the project resolves").into_parts().0
}

fn unresolved(body: &str) -> Report {
    member(body).expect_err("the project does not resolve")
}

/// workspaces.md "Explicit inheritance": every inheritable key takes its
/// value from the group its marker names.
#[test]
fn a_member_inherits_every_key_from_the_default_group() {
    let project = resolved(
        "name = \"member\"\n\
         publisher.workspace = true\n\
         version.workspace = true\n\
         sources = []\n\
         license.workspace = true\n\
         topics.workspace = true\n\
         metamodel.workspace = true\n\
         maintainers.workspace = true\n\
         urls.workspace = true\n",
    );
    assert_eq!(project.publisher.as_str(), "acme");
    assert_eq!(project.version.to_string(), "1.0.0");
    assert_eq!(
        project.license.as_ref().map(ToString::to_string).as_deref(),
        Some("MIT")
    );
    assert_eq!(project.topics, ["shared".to_owned()]);
    assert_eq!(
        project.metamodel.to_string(),
        "https://www.omg.org/spec/SysML/20250201"
    );
    assert_eq!(
        project
            .maintainers
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["Root Maintainer <root@example.com>".to_owned()]
    );
    assert_eq!(
        project.urls.website.as_ref().map(Iri::as_str),
        Some("https://example.com")
    );
}

/// "To inherit from a non-default group, `<key>.workspace =
/// \"group-name\"`."
#[test]
fn a_member_inherits_from_a_named_group() {
    let project = resolved(
        "name = \"member\"\n\
         publisher.workspace = \"legacy\"\n\
         version.workspace = \"legacy\"\n\
         sources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );
    assert_eq!(project.publisher.as_str(), "acme-legacy");
    assert_eq!(project.version.to_string(), "0.9.0");
}

/// workspaces.md: `urls` may be inherited as a whole table, which takes
/// the group's `urls` as it is.
#[test]
fn a_whole_urls_table_is_inherited() {
    let project = resolved(&format!("{RESOLVABLE}urls.workspace = true\n"));
    assert_eq!(
        project.urls.repository.as_ref().map(Iri::as_str),
        Some("https://example.com/repo.git")
    );
    assert_eq!(
        project.urls.documentation.as_ref().map(Iri::as_str),
        Some("https://example.com/docs")
    );
}

/// ... or key by key, mixed with keys the member writes itself.
#[test]
fn one_url_key_is_inherited_while_another_is_given() {
    let project = resolved(&format!(
        "{RESOLVABLE}\
         urls.website.workspace = \"legacy\"\n\
         urls.repository = \"https://member.example.com/repo.git\"\n"
    ));
    assert_eq!(
        project.urls.website.as_ref().map(Iri::as_str),
        Some("https://legacy.example.com")
    );
    assert_eq!(
        project.urls.repository.as_ref().map(Iri::as_str),
        Some("https://member.example.com/repo.git")
    );
    assert!(project.urls.documentation.is_none());
}

/// workspaces.md: "A project that
/// is not a member is outside every workspace regardless of where its
/// directory sits."
#[test]
fn a_marker_outside_a_workspace_is_an_error() {
    let report = manifest(
        "name = \"member\"\npublisher.workspace = true\nversion = \"1.0.0\"\nsources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    )
    .resolve(None)
    .expect_err("no workspace");
    assert_eq!(
        texts(&report),
        [
            "`publisher` inherits from the default group, but the project is not a member of any workspace"
        ]
    );

    // An opt-in usage names itself rather than a key.
    let text = format!(
        "[project]\n{RESOLVABLE}\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\nworkspace = \"legacy\"\n"
    );
    let report = read(&text)
        .expect("validates")
        .into_parts()
        .0
        .resolve(None)
        .expect_err("no workspace");
    assert_eq!(
        texts(&report),
        [
            "usage `acme/lib` inherits from group `legacy`, but the project is not a member of any workspace"
        ]
    );
}

/// workspaces.md: "A marker must find a value: naming a group that
/// does not exist ... is an error."
#[test]
fn a_marker_naming_a_group_that_does_not_exist_is_an_error() {
    let report = unresolved(&format!("{RESOLVABLE}license.workspace = \"nope\"\n"));
    assert_eq!(
        texts(&report),
        ["`license` inherits from group `nope`, which the workspace does not define"]
    );
}

/// "... or one that does not define the key". A group that gives the key
/// an *empty* value does define it: `topics = []` satisfies a marker,
/// while no `topics` at all does not.
#[test]
fn a_marker_on_a_key_the_group_does_not_give_is_an_error() {
    let report = unresolved(&format!("{RESOLVABLE}topics.workspace = \"legacy\"\n"));
    assert_eq!(
        texts(&report),
        ["`topics` inherits from group `legacy`, which does not give it"]
    );

    let empty = workspace("[workspace]\nmembers = []\ntopics = []\n");
    let project = manifest(&format!("{RESOLVABLE}topics.workspace = true\n"))
        .resolve(Some(&empty))
        .expect("an empty list is a value")
        .into_parts()
        .0;
    assert_eq!(project.topics, [] as [std::string::String; 0]);
}

/// Resolution accumulates, as every other validation here does.
#[test]
fn every_failing_marker_is_reported() {
    let report = unresolved(
        "name = \"member\"\n\
         publisher.workspace = \"nope\"\n\
         version.workspace = \"nope\"\n\
         sources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\
         topics.workspace = \"legacy\"\n",
    );
    assert_eq!(texts(&report).len(), 3, "unexpected report: {report}");
    assert!(is_ascending(&places(&report)), "{:?}", places(&report));
}

/// workspaces.md "Common usages": an opt-in "takes the matching group
/// entry whole", and a group entry's `dir` is relative to the workspace
/// root "whichever member opts into it".
#[test]
fn an_opt_in_takes_the_group_entry_whole() {
    let text = format!(
        "[project]\n{RESOLVABLE}\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"shared-dep\"\nworkspace = true\n\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"legacy-dep\"\nworkspace = \"legacy\"\n\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"own\"\ndir = \"../own\"\n"
    );
    let project = read(&text)
        .expect("validates")
        .into_parts()
        .0
        .resolve(Some(&workspace(WORKSPACE)))
        .expect("resolves")
        .into_parts()
        .0;
    assert_eq!(project.usages.len(), 3);
    assert!(matches!(
        &project.usages[0].usage,
        Usage::Index { name, version_constraint, publisher: _ }
            if name.as_str() == "shared-dep" && version_constraint.to_string() == "^2.0"
    ));
    assert_eq!(project.usages[1].base, UsageBase::WorkspaceRoot);
    assert_eq!(
        project.usages[1].resolve_local_path(
            camino::Utf8Path::new("/repo/projects/a"),
            Some(camino::Utf8Path::new("/repo")),
        ),
        Some(camino::Utf8PathBuf::from("/repo/legacy/dep"))
    );
    assert_eq!(project.usages[2].base, UsageBase::Project);
    assert_eq!(
        project.usages[2].resolve_local_path(
            camino::Utf8Path::new("/repo/projects/a"),
            Some(camino::Utf8Path::new("/repo")),
        ),
        Some(camino::Utf8PathBuf::from("/repo/projects/a/../own"))
    );
}

/// model-toml.md "Normalization": group matching is one of the literal
/// comparisons, "so an opt-in entry has to spell publisher and name
/// exactly as the group entry does".
#[test]
fn an_opt_in_matches_publisher_and_name_literally() {
    let text = format!(
        "[project]\n{RESOLVABLE}\n\
         [[inline-usages]]\npublisher = \"Acme\"\nname = \"shared-dep\"\nworkspace = true\n"
    );
    let report = read(&text)
        .expect("validates")
        .into_parts()
        .0
        .resolve(Some(&workspace(WORKSPACE)))
        .expect_err("`Acme` is not `acme`");
    assert_eq!(
        texts(&report),
        [
            "usage `Acme/shared-dep` inherits from the default group, which has no entry with \
             that publisher and name"
        ]
    );
}

/// "An opt-in must find its entry: a marker naming a group that does not
/// exist ... is an error, as for inherited metadata."
#[test]
fn an_opt_in_naming_a_group_that_does_not_exist_is_an_error() {
    let text = format!(
        "[project]\n{RESOLVABLE}\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\nworkspace = \"nope\"\n"
    );
    let report = read(&text)
        .expect("validates")
        .into_parts()
        .0
        .resolve(Some(&workspace(WORKSPACE)))
        .expect_err("no such group");
    assert_eq!(
        texts(&report),
        ["usage `acme/lib` inherits from group `nope`, which the workspace does not define"]
    );
}

/// model-toml.md `sources`: "Always required for the OMG metamodel ... a
/// project that has none is written as `sources = []`." Checked after
/// inheritance, since the metamodel may itself be inherited.
#[test]
fn sources_is_required_after_inheritance() {
    let report = manifest(&format!(
        "{REQUIRED}metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
    ))
    .resolve(None)
    .expect_err("no sources");
    assert_eq!(
        texts(&report),
        [
            "missing key `sources` in `[project]`; the OMG metamodel requires it\n\
             (a project with no source files writes `sources = []`)"
        ]
    );

    let project = manifest(RESOLVABLE)
        .resolve(None)
        .expect("an empty list is a value")
        .into_parts()
        .0;
    assert!(project.sources.is_empty());
}

/// A Syside project needs `sources` as an OMG one does, and the message
/// says where `root` fits.
#[test]
fn a_syside_project_without_sources_names_the_key_root_lives_in() {
    let report = standalone(&syside("", "root = \"main.sysml\"\n")).expect_err("no sources");
    assert_eq!(texts(&report), [SYSIDE_SOURCES_MISSING]);
}

/// An unrecognized metamodel never reaches the metamodel-dependent
/// checks: the file is refused when it is read, so a missing `sources`
/// is a second run's problem. The bargain `manifest-version` already
/// makes -- what the file says about its own schema cannot be acted on,
/// so nothing that depends on it can be judged.
#[test]
fn an_unrecognized_metamodel_hides_the_sources_check() {
    let report = rejected(&format!(
        "{REQUIRED}metamodel = \"https://example.com/future\"\n"
    ));
    assert_eq!(texts(&report).len(), 1, "{:?}", texts(&report));
    assert!(
        texts(&report)[0].contains("is not one of the four values"),
        "{:?}",
        texts(&report)
    );
}

/// model-toml.md "Standard library": `stdlib` "is refused unless the
/// project's publisher normalizes to `omg` and its metamodel, declared or
/// inherited, is a recognized OMG one (the IRI needs its release date)".
#[test]
fn stdlib_is_refused_unless_the_project_is_the_standard_library() {
    let stdlib = |publisher: &str, metamodel: &str| {
        // A Syside project needs its `root`; this test is about
        // `stdlib`, so the fixture supplies one.
        let root = if metamodel.contains("sysand.com") {
            "root = \"Base.kerml\"\n"
        } else {
            ""
        };
        manifest(&format!(
            "publisher = \"{publisher}\"\nname = \"Kernel Semantic Library\"\n\
             version = \"1.0.0\"\nsources = [\"Base.kerml\"]\nstdlib = \"Semantic-Library\"\n\
             metamodel = \"{metamodel}\"\n{root}"
        ))
        .resolve(None)
    };

    // The publisher is compared normalized, so every spelling passes.
    for publisher in ["OMG", "omg", "Omg"] {
        let project = stdlib(publisher, "https://www.omg.org/spec/KerML/20250201")
            .expect("the standard library")
            .into_parts()
            .0;
        assert_eq!(project.stdlib.expect("stdlib").as_str(), "Semantic-Library");
    }

    for (publisher, metamodel, found) in [
        (
            "acme",
            "https://www.omg.org/spec/KerML/20250201",
            "found publisher `acme` and the OMG metamodel \
             `https://www.omg.org/spec/KerML/20250201`",
        ),
        (
            "OMG",
            "https://www.sysand.com/spec/KerML/20250201",
            "found publisher `OMG` and the Syside metamodel \
             `https://www.sysand.com/spec/KerML/20250201`",
        ),
    ] {
        let report = stdlib(publisher, metamodel).expect_err("refused");
        assert_eq!(
            texts(&report),
            [format!(
                "`stdlib` is only allowed for the standard library itself: a publisher \
                 normalizing to `omg` and an OMG metamodel;\n{found}"
            )
            .as_str()],
            "{publisher} / {metamodel}"
        );
    }
}

/// The gate needs both a publisher and a metamodel; with either missing
/// there is nothing to judge, and a second diagnostic about a value that
/// is not there would only obscure the first.
#[test]
fn stdlib_is_not_checked_when_the_publisher_did_not_resolve() {
    let report = unresolved(
        "name = \"Kernel Semantic Library\"\n\
         publisher.workspace = \"nope\"\n\
         version = \"1.0.0\"\n\
         sources = []\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n\
         stdlib = \"Semantic-Library\"\n",
    );
    assert_eq!(
        texts(&report),
        ["`publisher` inherits from group `nope`, which the workspace does not define"]
    );
}

/// The check that waits for inheritance really does wait for it: the
/// metamodel that decides the message comes from the group.
#[test]
fn an_inherited_metamodel_settles_the_sources_check() {
    let syside = workspace(
        "[workspace]\nmembers = []\nmetamodel = \"https://www.sysand.com/spec/SysML/20250201\"\n",
    );
    let report = manifest(&format!(
        "{REQUIRED}metamodel.workspace = true\nroot = \"main.sysml\"\n"
    ))
    .resolve(Some(&syside))
    .expect_err("no sources");
    assert_eq!(
        texts(&report),
        [SYSIDE_SOURCES_MISSING],
        "unexpected report: {report}"
    );
}

/// workspaces.md: a marker-free non-member "reads as an ordinary
/// standalone project", so a caller has to be able to ask before it
/// resolves.
#[test]
fn inherits_answers_before_resolving() {
    assert!(!manifest(RESOLVABLE).inherits());
    assert!(manifest(&format!("{RESOLVABLE}urls.workspace = true\n")).inherits());
    assert!(manifest(&format!("{RESOLVABLE}urls.website.workspace = true\n")).inherits());
}

/// A `project` key that is written but is not a table has one thing
/// wrong with it. Reading an empty table on top of the type error would
/// report `publisher`, `name` and `version` missing as well -- four
/// complaints for one mistake.
#[test]
fn a_project_that_is_not_a_table_is_one_diagnostic() {
    let report = read("project = 3\n").expect_err("not a table");
    assert_eq!(
        texts(&report),
        ["`project` in the top level must be a table, but it is an integer"]
    );
}

/// A table built from values, as a command that writes a manifest
/// builds one.
fn built() -> ProjectTable {
    ProjectTable::new(
        IndexPublisher::parse("acme").expect("a valid publisher"),
        IndexName::parse("widgets").expect("a valid name"),
        semver::Version::new(1, 0, 0),
    )
}

#[test]
fn a_project_table_can_be_built_from_values() {
    let table = built();

    assert_eq!(table.name.as_str(), "widgets");
    assert_eq!(
        table.publisher.provided().map(IndexPublisher::as_str),
        Some("acme")
    );
    assert_eq!(
        table.version.provided(),
        Some(&semver::Version::new(1, 0, 0))
    );
    // Everything a generated file only writes when it has been given
    // one. `sources` among them: see `a_built_manifest_without_sources`.
    assert!(table.sources.is_none());
    assert!(table.license.is_none());
    assert!(table.description.is_none());
    assert!(table.topics.is_none());
    assert!(table.metamodel.is_none());
    assert!(table.maintainers.is_none());
    assert!(table.stdlib.is_none());
    assert!(table.urls.is_none());
    assert!(table.root.is_none());
}

#[test]
fn a_built_manifest_resolves_like_one_that_was_read() {
    let mut project = built();
    project.sources =
        Some(Sources::new(vec!["Widgets.sysml".into()]).expect("a valid sources list"));
    // A resolved project declares a metamodel, so a table built in code
    // is given one too.
    project.metamodel = Some(MetaProperty::Provided(
        KnownMetamodel::recognize("https://www.omg.org/spec/SysML/20250201")
            .expect("one of the four"),
    ));

    let manifest = ProjectManifest {
        project,
        inline_usages: vec![],
    };
    let (resolved, warnings) = manifest
        .resolve(None)
        .expect("a built manifest resolves")
        .into_parts();

    assert_eq!(resolved.publisher.as_str(), "acme");
    assert_eq!(resolved.name.as_str(), "widgets");
    assert_eq!(resolved.version, semver::Version::new(1, 0, 0));
    assert_eq!(
        resolved.sources.as_slice(),
        &[typed_path::Utf8UnixPathBuf::from("Widgets.sysml")]
    );
    assert_eq!(
        resolved.metamodel.to_string(),
        "https://www.omg.org/spec/SysML/20250201"
    );
    assert_eq!(
        resolved.usages,
        [] as [crate::manifest::usage::LocatedUsage; 0]
    );
    assert_eq!(
        warnings,
        [] as [crate::manifest::diagnostics::Diagnostic; 0]
    );
}

#[test]
fn a_built_manifest_without_sources_reports_it_without_a_position() {
    let mut project = built();
    project.metamodel = Some(MetaProperty::Provided(
        KnownMetamodel::recognize("https://www.omg.org/spec/SysML/20250201")
            .expect("one of the four"),
    ));
    let manifest = ProjectManifest {
        project,
        inline_usages: vec![],
    };

    let report = manifest
        .resolve(None)
        .expect_err("a project with no `sources` does not resolve");

    // The diagnostic is the same one a file gets; what a built manifest
    // cannot have is the position, since the two fields that carry one
    // are private and only `validate` fills them.
    assert_eq!(texts(&report).len(), 1);
    assert!(texts(&report)[0].starts_with("missing key `sources` in `[project]`"));
    assert!(report.diagnostics()[0].position.is_none());
}

// -------------------------------------- the metamodel is mandatory

/// model-toml.md "Metamodel": "`metamodel` is a URL,
/// and in a `model.toml` it is mandatory."
#[test]
fn a_metamodel_is_required() {
    let report = manifest(&format!("{REQUIRED}sources = []\n"))
        .resolve(None)
        .expect_err("no metamodel");
    assert_eq!(
        texts(&report),
        [
            "missing key `metamodel` in `[project]`;\nevery project declares one of \
             `https://www.omg.org/spec/KerML/<date>`, `https://www.omg.org/spec/SysML/<date>`, \
             `https://www.sysand.com/spec/KerML/<date>` or \
             `https://www.sysand.com/spec/SysML/<date>`"
        ]
    );
}

/// "Exactly four values are recognized -- two families, two spec kinds
/// each." All four resolve, and the resolved project carries the kind
/// rather than the string it was written as.
#[test]
fn each_of_the_four_metamodels_resolves() {
    for (value, family, spec) in [
        (
            "https://www.omg.org/spec/KerML/20250201",
            MetamodelFamily::Omg,
            SpecKind::KerMl,
        ),
        (
            "https://www.omg.org/spec/SysML/20250201",
            MetamodelFamily::Omg,
            SpecKind::SysMl,
        ),
        (
            "https://www.sysand.com/spec/KerML/20250201",
            MetamodelFamily::Syside,
            SpecKind::KerMl,
        ),
        (
            "https://www.sysand.com/spec/SysML/20250201",
            MetamodelFamily::Syside,
            SpecKind::SysMl,
        ),
    ] {
        // A Syside project has no `sources` schema in v1, so the two
        // Syside values are checked through `validate` alone; the OMG
        // ones resolve whole.
        let manifest = manifest(&format!(
            "{REQUIRED}sources = []\nmetamodel = \"{value}\"\n"
        ));
        let declared = manifest
            .project
            .metamodel
            .as_ref()
            .and_then(|property| property.provided())
            .expect("a declared metamodel");
        let kind = declared;
        assert_eq!(kind.family, family, "{value}");
        assert_eq!(kind.spec, spec, "{value}");

        if family == MetamodelFamily::Omg {
            let project = manifest.resolve(None).expect("resolves").into_parts().0;
            assert_eq!(project.metamodel.to_string(), value);
        }
    }
}

/// workspaces.md: "`metamodel.workspace = true` [...] satisfies its
/// requirement in `[project]`" -- which is why the requirement is
/// checked after inheritance and not while the table is read.
#[test]
fn an_inherited_metamodel_satisfies_the_requirement() {
    let project = resolved(
        "name = \"member\"\npublisher.workspace = true\nversion = \"1.0.0\"\nsources = []\nmetamodel.workspace = true\n",
    );
    assert_eq!(
        project.metamodel.to_string(),
        "https://www.omg.org/spec/SysML/20250201"
    );
}

/// The metamodel's language rule through `resolve`, positioned at the
/// entry -- which is why the table keeps each entry's position: the
/// reader that could point at one is gone by the time the metamodel is
/// known.
#[test]
fn a_source_the_metamodel_does_not_admit_is_refused() {
    let report = unresolved(
        "name = \"member\"\npublisher = \"acme\"\nversion = \"1.0.0\"\n\
         sources = [\"a.kerml\", \"b.sysml\"]\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n",
    );
    assert_eq!(
        texts(&report),
        [
            "`sources` entry `b.sysml` is a SysML file, which the metamodel \
             `https://www.omg.org/spec/KerML/20250201` does not admit;\n\
             a KerML project lists `.kerml` files, a SysML project `.sysml` and `.kerml` files"
        ]
    );
    // The second entry, not the array and not the first.
    assert_eq!(places(&report), [(5, 23)]);
}

/// The check waits for inheritance, like the required-key one: a member
/// that inherits KerML may not list a `.sysml` file either.
#[test]
fn an_inherited_metamodel_decides_the_language() {
    let kerml = workspace(
        "[workspace]\nmembers = []\nmetamodel = \"https://www.omg.org/spec/KerML/20250201\"\n",
    );
    let report = manifest(
        "name = \"member\"\npublisher = \"acme\"\nversion = \"1.0.0\"\n\
         sources = [\"a.sysml\"]\nmetamodel.workspace = true\n",
    )
    .resolve(Some(&kerml))
    .expect_err("a SysML file under an inherited KerML metamodel");
    assert!(
        texts(&report)[0].contains(
            "which the metamodel `https://www.omg.org/spec/KerML/20250201` does not admit"
        ),
        "{:?}",
        texts(&report)
    );
}

// ------------------- opting into a standard-library group entry

/// A workspace whose default group carries the standard library in the
/// form workspaces.md shows: a publisher-less `kpar-url`.
const STDLIB_WORKSPACE: &str = "manifest-version = \"1.0.0\"

[workspace]
members = []
metamodel = \"https://www.omg.org/spec/KerML/20250201\"

[[workspace.inline-usages]]
name = \"Kernel Semantic Library\"
kpar-url = \"https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar\"
";

fn in_stdlib_workspace(body: &str) -> Result<Validated<ResolvedProject>, Report> {
    manifest(body).resolve(Some(&workspace(STDLIB_WORKSPACE)))
}

/// workspaces.md "Common usages": "an opt-in to such an entry likewise
/// gives only `name` and the marker", and takes the group entry whole.
#[test]
fn an_opt_in_takes_a_publisher_less_group_entry() {
    let project = in_stdlib_workspace(
        "publisher = \"acme\"\nname = \"member\"\nversion = \"1.0.0\"\nsources = []\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n\n\
         [[inline-usages]]\nname = \"Kernel Semantic Library\"\nworkspace = true\n",
    )
    .expect("the project resolves")
    .into_parts()
    .0;
    let usage = &project.usages[0].usage;
    assert!(usage.is_stdlib(), "{usage:?}");
    assert_eq!(usage.identity().name, "kernel-semantic-library");
}

/// "Entries are matched by publisher and name, compared literally", and
/// absence is part of the spelling: an opt-in that names a publisher
/// does not match an entry that declares none.
#[test]
fn an_opt_in_naming_a_publisher_misses_a_stdlib_entry() {
    let report = in_stdlib_workspace(
        "publisher = \"acme\"\nname = \"member\"\nversion = \"1.0.0\"\nsources = []\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n\n\
         [[inline-usages]]\npublisher = \"OMG\"\nname = \"Kernel Semantic Library\"\n\
         workspace = true\n",
    )
    .expect_err("the opt-in matches nothing");
    assert!(
        report.diagnostics().iter().any(|diagnostic| diagnostic
            .message
            .contains("has no entry with that publisher and name")),
        "{report:?}"
    );
}

/// And the other way: a publisher-less opt-in against a group that has
/// only ordinary entries.
#[test]
fn a_publisher_less_opt_in_misses_an_ordinary_entry() {
    let report = member(
        "publisher = \"acme\"\nname = \"member\"\nversion = \"1.0.0\"\nsources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\n\
         [[inline-usages]]\nname = \"shared-dep\"\nworkspace = true\n",
    )
    .expect_err("the opt-in matches nothing");
    assert!(
        report.diagnostics().iter().any(|diagnostic| diagnostic
            .message
            .contains("has no entry with that publisher and name")),
        "{report:?}"
    );
}
