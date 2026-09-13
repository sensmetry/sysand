// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use serde::{Deserialize, Serialize};
use typed_path::Utf8UnixPathBuf;

use crate::manifest::common::{
    InheritFrom, InheritMarker, Maintainer, Sources, SourcesError, StdlibStem, is_valid_group_name,
    normalized_identity,
};
use crate::model::{IndexName, IndexPublisher};

fn sources(entries: &[&str]) -> Result<Sources, SourcesError> {
    Sources::new(
        entries
            .iter()
            .map(|entry| Utf8UnixPathBuf::from(*entry))
            .collect(),
    )
}

fn sources_error(entries: &[&str]) -> SourcesError {
    sources(entries).expect_err("entries should be rejected")
}

/// model-toml.md "Main project metadata" gives the accepted syntax as
/// regexes; publishers do not admit `.` as a separator, names do.
#[test]
fn identifier_syntax_follows_the_documented_regexes() {
    for ok in ["acme", "Acme Corp", "a-b", "ab1"] {
        assert!(
            IndexPublisher::parse(ok).is_ok(),
            "{ok} should be a publisher"
        );
    }
    for bad in ["ab", "-acme", "acme-", "acme--corp", "acme.corp", "açme"] {
        assert!(
            IndexPublisher::parse(bad).is_err(),
            "{bad} should not be a publisher"
        );
    }
    IndexName::parse("foo.bar").unwrap();
    IndexName::parse("foo..bar").unwrap_err();
    IndexName::parse("a".repeat(51)).unwrap_err();

    let name = IndexName::parse("Widget Kit").expect("a valid name");
    assert_eq!(name.as_str(), "Widget Kit");
    assert_eq!(name.normalized(), "widget-kit");
}

#[test]
fn identifier_error_names_the_rule() {
    let publisher = IndexPublisher::parse("ab").expect_err("too short");
    assert_eq!(
        publisher.to_string(),
        "publisher must be 3-50 ASCII alphanumeric chars, with single ` ` or `-` separators between words"
    );

    let name = IndexName::parse("ab").expect_err("too short");
    assert_eq!(
        name.to_string(),
        "name must be 3-50 ASCII alphanumeric chars, with single ` `, `-`, or `.` separators between words"
    );
}

/// model-toml.md "Normalization": projects are resolved by their
/// normalized identity, which is also what the lockfile records.
#[test]
fn normalized_identity_is_a_pkg_sysand_purl() {
    let publisher = IndexPublisher::parse("Acme Corp").expect("a valid publisher");
    let name = IndexName::parse("Widget Kit").expect("a valid name");
    let identity = normalized_identity(&publisher, &name);

    assert_eq!(identity.to_string(), "acme-corp/widget-kit");
    assert_eq!(
        identity.lock_identifier(),
        "pkg:sysand/acme-corp/widget-kit"
    );
}

/// model-toml.md "Standard library": "A usage is a standard-library usage
/// iff its publisher normalizes to `omg`".
#[test]
fn stdlib_publisher_is_recognized_after_normalization() {
    for publisher in ["omg", "OMG", "Omg"] {
        assert!(
            IndexPublisher::parse(publisher)
                .expect("a valid publisher")
                .is_stdlib_publisher(),
            "{publisher} should be the standard-library publisher"
        );
    }
    assert!(
        !IndexPublisher::parse("omgx")
            .expect("a valid publisher")
            .is_stdlib_publisher()
    );
}

/// model-toml.md maintainer rule: each entry is a string of the form
/// `name <email>`, both parts mandatory.
#[test]
fn maintainer_string_must_match_name_email_format() {
    #[derive(Debug, Deserialize)]
    struct Wrapper {
        #[expect(dead_code)]
        maintainers: Vec<Maintainer>,
    }

    let error = toml_edit::de::from_str::<Wrapper>("maintainers = [\"no email here\"]\n")
        .expect_err("not a maintainer");
    assert!(
        error.to_string().contains("name <email>"),
        "unexpected error: {error}"
    );

    for bad in ["<a@b.c>", "A B <>", "A B", "A B <a@b.c> trailing"] {
        assert!(Maintainer::parse(bad).is_err(), "{bad} is not a maintainer");
    }

    let maintainer = Maintainer::parse("A B <a@b.c>").expect("a maintainer");
    assert_eq!(maintainer.name, "A B");
    assert_eq!(maintainer.email, "a@b.c");
    assert_eq!(maintainer.to_string(), "A B <a@b.c>");
}

/// model-toml.md "Standard library": the `stdlib` value is "the last
/// segment of the IRI at which the library's KPAR is published",
/// validated as a single path segment so it cannot carry `/` or `..` into
/// an IRI.
#[test]
fn stdlib_key_is_a_path_segment() {
    for ok in ["Kernel-Libraries", "Systems-Library", "a1"] {
        assert!(StdlibStem::parse(ok).is_ok(), "{ok} should be a stem");
    }
    for bad in ["", "-x", "a/b", "..", "a b", "a.kpar"] {
        assert!(
            StdlibStem::parse(bad).is_err(),
            "{bad} should not be a stem"
        );
    }
    assert_eq!(
        StdlibStem::parse("a/b")
            .expect_err("not a stem")
            .to_string(),
        "`stdlib` value `a/b` is not a single IRI path segment (`[A-Za-z0-9][A-Za-z0-9-]*`)"
    );
}

/// workspaces.md "Explicit inheritance": `<key>.workspace = true` selects
/// the implicit default group, `<key>.workspace = "group-name"` a named
/// one.
#[test]
fn project_inheritance_markers() {
    #[derive(Debug, Deserialize, Serialize)]
    struct Wrapper {
        version: InheritFrom,
    }

    let default: Wrapper =
        toml_edit::de::from_str("version = { workspace = true }\n").expect("parses");
    assert_eq!(default.version, InheritFrom::DefaultGroup);
    assert_eq!(
        toml_edit::ser::to_string(&default).expect("serializes"),
        "version = { workspace = true }\n"
    );

    let group: Wrapper =
        toml_edit::de::from_str("version = { workspace = \"legacy\" }\n").expect("parses");
    assert_eq!(group.version, InheritFrom::Group("legacy".to_owned()));
    assert_eq!(
        toml_edit::ser::to_string(&group).expect("serializes"),
        "version = { workspace = \"legacy\" }\n"
    );
    assert_eq!(group.version.to_string(), "group `legacy`");
    assert_eq!(InheritFrom::DefaultGroup.to_string(), "the default group");
}

/// workspaces.md: there is no "inherit nothing" marker; a key that is not
/// inherited carries its own value.
#[test]
fn workspace_false_is_not_a_marker() {
    let error = InheritFrom::try_from(InheritMarker::Flag(false)).expect_err("not a marker");
    assert_eq!(
        error.to_string(),
        "`workspace = false` is not allowed; a key that is not inherited carries its own value"
    );

    let invalid =
        InheritFrom::try_from(InheritMarker::Group("bad name".to_owned())).expect_err("not a name");
    assert_eq!(
        invalid.to_string(),
        "`bad name` is not a valid workspace group name (a bare TOML key: letters, digits, \
         `-` and `_`)"
    );
}

/// workspaces.md "Explicit inheritance": "`<group-name>` is a bare TOML
/// key: letters, digits, `-` and `_`."
#[test]
fn group_names_are_bare_toml_keys() {
    for ok in ["a_b-1", "legacy", "A1"] {
        assert!(is_valid_group_name(ok), "{ok} should be a group name");
    }
    for bad in ["a b", "a.b", "", "a/b"] {
        assert!(
            !is_valid_group_name(bad),
            "{bad} should not be a group name"
        );
    }
}

/// model-toml.md `sources`: entries are relative, `/`-separated, and are
/// stored normalized, which is what the KPAR maps are keyed by.
#[test]
fn sources_entries_are_normalized_and_stay_inside_the_project_root() {
    for (entries, expected) in [
        (&["/abs/main.sysml"][..], "is absolute"),
        (&["../outside.sysml"], "escapes the project root"),
        (&["src/../../outside.sysml"], "escapes the project root"),
        (&["."], "is the project root, not a source file"),
        (&["src/"], "names a directory"),
        (
            &["src/main.sysml", "src/main.sysml"],
            "is listed more than once",
        ),
        // Two spellings of one file are one file, and therefore a duplicate.
        (&["./main.sysml", "main.sysml"], "name the same file"),
        (&["src//main.sysml", "src/main.sysml"], "name the same file"),
    ] {
        let error = sources_error(entries);
        assert!(
            error.to_string().contains(expected),
            "{entries:?}: unexpected error: {error}"
        );
    }

    let stored = sources(&["./main.sysml", "src/../parts.kerml", "lib//util.kerml"])
        .expect("all three are inside the project root");
    assert_eq!(
        stored.as_slice(),
        [
            Utf8UnixPathBuf::from("main.sysml"),
            Utf8UnixPathBuf::from("parts.kerml"),
            Utf8UnixPathBuf::from("lib/util.kerml"),
        ]
    );
}

/// A manifest is read the same way on every platform, so a Windows path
/// is not a source path (see `utils::normalize_relative`).
#[test]
fn sources_rejects_windows_paths() {
    assert!(
        sources_error(&["C:/main.sysml"])
            .to_string()
            .contains("is absolute")
    );
    assert!(
        sources_error(&["src\\main.sysml"])
            .to_string()
            .contains("backslash")
    );
}

/// model-toml.md `sources`: "Every entry names a KerML (`.kerml`) or SysML
/// (`.sysml`) file, the extension compared case-insensitively".
#[test]
fn sources_entries_must_be_model_files() {
    for ok in &["a.sysml", "A.SYSML", "b.KerML", "src/c.kerml"] {
        assert!(sources(&[ok]).is_ok(), "{ok} should be a source file");
    }
    for bad in &["readme.md", "LICENSE", "a.sysml.bak", "sources"] {
        assert!(
            sources(&[bad]).is_err(),
            "{bad} should not be a source file"
        );
    }
    assert_eq!(
        sources_error(&["readme.md"]).to_string(),
        "`sources` entry `readme.md` is not a KerML (`.kerml`) or SysML (`.sysml`) file; \
         `sources` lists model files only"
    );
}

/// model-toml.md: `sources` "cannot be inherited by a member ..., since
/// the file layout of one project says nothing about another's".
#[test]
fn sources_cannot_be_inherited() {
    #[derive(Debug, Deserialize)]
    struct Wrapper {
        sources: Sources,
    }

    for marker in [
        "sources.workspace = true",
        "sources.workspace = \"legacy\"",
        "sources.workspace = false",
        "sources = { workspace = true }",
    ] {
        let error = toml_edit::de::from_str::<Wrapper>(&format!("{marker}\n"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("cannot be inherited"),
            "{marker}: unexpected error: {error}"
        );
    }

    let parsed: Wrapper =
        toml_edit::de::from_str("sources = [\"a.sysml\"]\n").expect("a plain list still parses");
    assert_eq!(parsed.sources.as_slice().len(), 1);
}

#[test]
fn sources_serializes_the_normalized_form() {
    #[derive(Debug, Serialize)]
    struct Wrapper {
        sources: Sources,
    }

    let wrapper = Wrapper {
        sources: sources(&["./a.sysml", "src/../b.kerml"]).expect("valid sources"),
    };
    assert_eq!(
        toml_edit::ser::to_string(&wrapper).expect("serializes"),
        "sources = [\"a.sysml\", \"b.kerml\"]\n"
    );
}
