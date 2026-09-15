// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use std::assert_matches;

use super::{LocalSrcError, LocalSrcProject, ProjectFormat, detect_format};
use crate::model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw};
use crate::project::{ProjectMut as _, ProjectRead as _};

fn write_project_json(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join(".project.json"), content).expect("write .project.json");
}

fn write_meta_json(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join(".meta.json"), content).expect("write .meta.json");
}

fn write_model_toml(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join("model.toml"), content).expect("write model.toml");
}

fn read_error(dir: &camino::Utf8Path) -> String {
    LocalSrcProject::new_access(dir, None)
        .get_project()
        .expect_err("the project should not be readable")
        .to_string()
}

/// A key a manifest must carry is named by Sysand, not by serde.
#[test]
fn meta_json_without_index_names_the_key() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"p","version":"1.0.0"}"#);
    write_meta_json(dir.path(), r#"{"created":"2026-01-01T00:00:00Z"}"#);

    let error = read_error(dir.path());
    assert!(
        error.contains("missing key `index` in `.meta.json`"),
        "unexpected error: {error}"
    );
    assert!(
        !error.contains("missing field"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[test]
fn project_json_without_name_names_every_missing_key() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"publisher":"acme"}"#);

    let error = read_error(dir.path());
    assert!(
        error.contains("missing key `name` in `.project.json`"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("missing key `version` in `.project.json`"),
        "unexpected error: {error}"
    );
    Ok(())
}

/// The metadata is read first, so its metamodel can be named as a reason
/// for a key the information document does not carry.
#[test]
fn an_unrecognized_metamodel_is_named_as_a_possible_reason()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"publisher":"acme"}"#);
    write_meta_json(
        dir.path(),
        r#"{"index":{},"created":"2026-01-01T00:00:00Z","metamodel":"https://example.com/x"}"#,
    );

    let error = read_error(dir.path());
    assert!(
        error.contains("this may be because the project uses a metamodel"),
        "unexpected error: {error}"
    );
    Ok(())
}

/// A document that is not json at all is still serde's to report: it
/// knows where the text stops making sense.
#[test]
fn a_manifest_that_is_not_json_still_fails_through_serde() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempdir()?;
    write_project_json(dir.path(), "not json at all");

    let error = read_error(dir.path());
    assert!(
        error.contains("failed to deserialize"),
        "unexpected error: {error}"
    );
    Ok(())
}

/// An unknown key does not stop a read; it is reported and dropped.
#[test]
fn an_unknown_key_does_not_stop_a_read() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(
        dir.path(),
        r#"{"name":"p","version":"1.0.0","x-unknown":true}"#,
    );

    let (info, _) = LocalSrcProject::new_access(dir.path(), None).get_project()?;
    assert_eq!(info.expect("a project").name, "p");
    Ok(())
}

// ----- which manifest a directory holds -----

#[test]
fn an_empty_directory_has_neither_format() {
    let dir = tempdir().expect("a temporary directory");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Neither
    );
}

#[test]
fn a_project_json_alone_is_the_json_format() {
    let dir = tempdir().expect("a temporary directory");
    write_project_json(dir.path(), "{}");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Json
    );
}

/// The json side is *either* file, not both: a directory holding only a
/// `.meta.json` is a broken json project, not a directory without one.
#[test]
fn a_meta_json_alone_is_the_json_format() {
    let dir = tempdir().expect("a temporary directory");
    write_meta_json(dir.path(), "{}");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Json
    );
}

#[test]
fn a_model_toml_alone_is_the_toml_format() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = 1\n");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Toml
    );
}

#[test]
fn a_directory_with_both_is_ambiguous() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = 1\n");
    write_project_json(dir.path(), "{}");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Both
    );
}

/// model-toml.md "Mixed manifests": "current project has toml and json
/// configs: error out that it's unclear if the project is meant to be a
/// non-zipped KPAR, or dev project".
#[test]
fn an_ambiguous_directory_is_refused_when_it_is_read() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = 1\n");
    write_project_json(dir.path(), "{}");
    write_meta_json(dir.path(), "{}");
    assert_eq!(
        read_error(dir.path()),
        format!(
            "`{}` has both `model.toml` and `.project.json`;\n         \
             it is unclear whether it is a non-zipped KPAR or a development project",
            dir.path()
        )
    );
}

/// The message names the json file that is actually there.
#[test]
fn an_ambiguous_directory_names_the_json_file_it_has() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = 1\n");
    write_meta_json(dir.path(), "{}");
    assert!(
        read_error(dir.path()).starts_with(&format!(
            "`{}` has both `model.toml` and `.meta.json`;",
            dir.path()
        )),
        "unexpected message: {}",
        read_error(dir.path())
    );
}

// ----- reading a `model.toml` -----

/// The three keys `[project]` requires plus the `sources` an
/// OMG-metamodel project needs.
const TOML_REQUIRED: &str = "\
manifest-version = 1

[project]
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
";

fn toml_project(dir: &camino::Utf8Path) -> LocalSrcProject {
    LocalSrcProject::new_access(dir, None)
}

#[test]
fn a_model_toml_project_reads_as_the_pair_a_build_would_write() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        "manifest-version = 1\n\n\
         [project]\n\
         publisher = \"acme\"\n\
         name = \"widgets\"\n\
         version = \"1.2.3\"\n\
         sources = [\"a.sysml\"]\n\
         description = \"A description.\"\n\
         license = \"MIT\"\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );
    std::fs::write(dir.path().join("a.sysml"), "part def A;").expect("write a.sysml");

    let (info, meta) = toml_project(dir.path()).get_project().expect("readable");
    let info = info.expect("`.project.json`");
    let meta = meta.expect("`.meta.json`");
    assert_eq!(info.name, "widgets");
    assert_eq!(info.publisher.as_deref(), Some("acme"));
    assert_eq!(info.version, "1.2.3");
    assert_eq!(info.description.as_deref(), Some("A description."));
    assert_eq!(info.license.as_deref(), Some("MIT"));
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.omg.org/spec/SysML/20250201")
    );
}

#[test]
fn a_model_toml_projects_sources_are_indexed_and_hashed() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        &TOML_REQUIRED.replace("sources = []", "sources = [\"a.sysml\"]"),
    );
    std::fs::write(dir.path().join("a.sysml"), "part def A;").expect("write a.sysml");

    let meta = toml_project(dir.path())
        .get_meta()
        .expect("readable")
        .expect("`.meta.json`");
    assert_eq!(meta.index["A"], "a.sysml");
    let checksum = &meta.checksum.expect("checksums")["a.sysml"];
    assert_eq!(
        checksum.value,
        crate::utils::sha256_lowercase_hex("part def A;")
    );
    assert_eq!(checksum.algorithm, "SHA256");
}

/// A view's `created` is not the time it was taken: the project checksum
/// covers it, and that checksum reaches the lockfile.
#[test]
fn a_model_toml_project_has_a_fixed_created() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), TOML_REQUIRED);
    let meta = toml_project(dir.path())
        .get_meta()
        .expect("readable")
        .expect("`.meta.json`");
    assert_eq!(meta.created, "1970-01-01T00:00:00Z");
}

/// The property the fixed `created` exists for. Asserted across two
/// project values, since one would answer from its cache either way.
///
/// This one cannot enforce the decision on its own: `format_created`
/// truncates to whole seconds, so two reads inside the same second would
/// agree even against a clock. `a_model_toml_project_has_a_fixed_created`
/// is what pins it; this records what the pinning is *for*.
#[test]
fn two_reads_of_one_model_toml_project_agree_on_its_checksum() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        &TOML_REQUIRED.replace("sources = []", "sources = [\"a.sysml\"]"),
    );
    std::fs::write(dir.path().join("a.sysml"), "part def A;").expect("write a.sysml");

    let first = toml_project(dir.path())
        .checksum_canonical_hex()
        .expect("readable");
    let second = toml_project(dir.path())
        .checksum_canonical_hex()
        .expect("readable");
    assert!(first.is_some());
    assert_eq!(first, second);
}

/// The cache is observable only through what it stops happening:
/// `get_project` is what reads the sources, so an uncached second read
/// would fail on a file that is gone.
#[test]
fn a_model_toml_project_is_read_once() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        &TOML_REQUIRED.replace("sources = []", "sources = [\"a.sysml\"]"),
    );
    std::fs::write(dir.path().join("a.sysml"), "part def A;").expect("write a.sysml");

    let project = toml_project(dir.path());
    let first = project.get_project().expect("readable");
    std::fs::remove_file(dir.path().join("a.sysml")).expect("remove a.sysml");
    assert_eq!(project.get_project().expect("cached"), first);
}

#[test]
fn a_model_toml_that_is_not_toml_names_the_file() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "this is not toml\n");
    assert!(
        read_error(dir.path()).contains("`model.toml` is not valid TOML"),
        "unexpected message: {}",
        read_error(dir.path())
    );
}

/// A workspace configuration is a valid `model.toml` and not a project.
#[test]
fn a_model_toml_that_defines_a_workspace_is_not_a_project() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\n",
    );
    assert!(
        read_error(dir.path()).contains("`model.toml` defines a workspace, not a project"),
        "unexpected message: {}",
        read_error(dir.path())
    );
}

#[test]
fn a_model_toml_missing_a_required_key_reports_every_one() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        "manifest-version = 1\n\n[project]\nsources = []\n",
    );
    let error = read_error(dir.path());
    for key in ["publisher", "name", "version"] {
        assert!(
            error.contains(&format!("missing key `{key}` in `[project]`")),
            "`{key}` is not reported in: {error}"
        );
    }
}

/// A member's manifest read on its own, with no workspace to inherit
/// from, says so.
#[test]
fn a_model_toml_that_inherits_outside_a_workspace_says_so() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        "manifest-version = 1\n\n\
         [project]\n\
         name = \"widgets\"\n\
         version = \"1.2.3\"\n\
         publisher.workspace = true\n\
         sources = []\n",
    );
    assert!(
        read_error(dir.path()).contains("not a member of any workspace"),
        "unexpected message: {}",
        read_error(dir.path())
    );
}

/// `sources` does not check that its files exist -- that is "only
/// checked for build/publish" -- so a missing one is found when the view
/// is built, and the message is the reader's own.
#[test]
fn a_source_that_cannot_be_read_names_the_file_on_disk() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        &TOML_REQUIRED.replace("sources = []", "sources = [\"gone.sysml\"]"),
    );
    assert!(
        read_error(dir.path()).contains("gone.sysml"),
        "unexpected message: {}",
        read_error(dir.path())
    );
}

#[test]
fn a_source_that_does_not_parse_is_reported() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        &TOML_REQUIRED.replace("sources = []", "sources = [\"a.sysml\"]"),
    );
    std::fs::write(dir.path().join("a.sysml"), "package Foo { /*").expect("write a.sysml");
    assert!(
        read_error(dir.path()).starts_with("failed to extract symbol names from `a.sysml`"),
        "unexpected message: {}",
        read_error(dir.path())
    );
}

// ----- the writers a `model.toml` project refuses -----

#[test]
fn a_model_toml_project_refuses_a_json_info_write() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), TOML_REQUIRED);
    let error = toml_project(dir.path())
        .put_info(
            &InterchangeProjectInfoRaw::minimal("w".into(), "1.0.0".into()),
            true,
        )
        .expect_err("a json write into a toml project");
    assert_eq!(
        error.to_string(),
        format!(
            "`{}` is a `model.toml` project;\n         \
             `.project.json` cannot be written into one",
            dir.path()
        )
    );
}

#[test]
fn a_model_toml_project_refuses_a_json_meta_write() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), TOML_REQUIRED);
    assert_matches!(
        toml_project(dir.path())
            .put_meta(&InterchangeProjectMetadataRaw::default(), true)
            .expect_err("a json write into a toml project"),
        LocalSrcError::JsonWriteToTomlProject { dir: _, file: _ }
    );
}

/// The reason the guard exists. `do_include` reads the metadata --
/// which the view supplies -- and writes it straight back, so
/// without the guard it would leave a `.meta.json` beside the
/// `model.toml` and make the directory unreadable.
#[test]
fn including_a_file_does_not_turn_a_toml_project_into_both() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), TOML_REQUIRED);
    std::fs::write(dir.path().join("a.sysml"), "part def A;").expect("write a.sysml");

    let mut project = toml_project(dir.path());
    crate::commands::include::do_include(
        &mut project,
        [typed_path::Utf8UnixPathBuf::from("a.sysml")].into_iter(),
        true,
        true,
        None,
    )
    .expect_err("including into a toml project");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Toml
    );
}

// ----- which format a project says it is -----

#[test]
fn a_json_project_reports_the_json_format() {
    let dir = tempdir().expect("a temporary directory");
    write_project_json(dir.path(), "{}");
    assert_eq!(
        toml_project(dir.path()).format().expect("readable"),
        ProjectFormat::Json
    );
}

#[test]
fn a_model_toml_project_reports_the_toml_format() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), TOML_REQUIRED);
    assert_eq!(
        toml_project(dir.path()).format().expect("readable"),
        ProjectFormat::Toml
    );
}

/// The case the default answer would get wrong: an editable project
/// wraps the project a `dir` usage resolves to, so taking the default
/// would report json for a `model.toml` target.
#[test]
fn an_editable_project_reports_the_format_of_what_it_wraps() {
    use crate::project::editable::EditableProject;

    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), TOML_REQUIRED);
    let wrapped = EditableProject::new(
        typed_path::Utf8UnixPathBuf::from("lib"),
        toml_project(dir.path()),
    );
    assert_eq!(wrapped.format().expect("readable"), ProjectFormat::Toml);
}

// ----- the manifest behind the view -----

#[test]
fn reading_from_text_gives_what_reading_from_disk_gives() {
    let dir = tempdir().expect("a temporary directory");
    let text = TOML_REQUIRED.replace("sources = []", "sources = [\"a.sysml\"]");
    write_model_toml(dir.path(), &text);
    std::fs::write(dir.path().join("a.sysml"), "part def A;").expect("write a.sysml");

    let from_disk = toml_project(dir.path()).get_project().expect("readable");
    let from_text = crate::manifest::kpar_view::of_model_toml(&text, |source| {
        std::fs::read_to_string(dir.path().join(source.as_str())).map_err(|e| e.to_string())
    })
    .expect("readable");
    assert_eq!(from_disk, (Some(from_text.0), Some(from_text.1)));
}

#[test]
fn a_model_toml_project_gives_its_manifest() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        &TOML_REQUIRED.replace("sources = []", "sources = [\"a.sysml\"]"),
    );
    let manifest = toml_project(dir.path()).manifest().expect("a manifest");
    assert_eq!(manifest.publisher.as_str(), "acme");
    assert_eq!(manifest.name.as_str(), "widgets");
    assert_eq!(
        manifest
            .sources
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>(),
        ["a.sysml"]
    );
}

/// The reason the seam exists: a `.project.json` has no room for some of
/// what a `model.toml` says, so a caller that needs it comes here.
/// model-toml.md: `urls.repository` and `urls.documentation` "have no
/// counterpart in `.project.json` and are dropped".
#[test]
fn a_manifest_carries_what_the_view_drops() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        &format!(
            "{TOML_REQUIRED}\n\
             [project.urls]\n\
             website = \"https://example.com/\"\n\
             documentation = \"https://example.com/docs\"\n\
             repository = \"https://example.com/git\"\n"
        ),
    );
    let project = toml_project(dir.path());

    let manifest = project.manifest().expect("a manifest");
    assert_eq!(
        manifest
            .urls
            .repository
            .as_ref()
            .map(fluent_uri::Iri::as_str),
        Some("https://example.com/git")
    );
    assert_eq!(
        manifest
            .urls
            .documentation
            .as_ref()
            .map(fluent_uri::Iri::as_str),
        Some("https://example.com/docs")
    );

    let rendered = serde_json::to_string(&project.get_info().expect("readable").expect("info"))
        .expect("serializes");
    assert!(!rendered.contains("example.com/git"), "{rendered}");
    assert!(!rendered.contains("example.com/docs"), "{rendered}");
}

#[test]
fn a_json_project_has_no_toml_manifest() {
    let dir = tempdir().expect("a temporary directory");
    write_project_json(
        dir.path(),
        r#"{"name":"widgets","publisher":"acme","version":"1.2.3"}"#,
    );
    let error = toml_project(dir.path())
        .manifest()
        .expect_err("a json project");
    assert_eq!(
        error.to_string(),
        "this project has no `model.toml` manifest"
    );
}

#[test]
fn a_project_that_cannot_be_read_reports_its_own_error() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "this is not toml\n");
    let error = toml_project(dir.path())
        .manifest()
        .expect_err("not a manifest");
    assert_matches!(error, crate::project::ManifestError::Project(_));
}

#[test]
fn an_editable_project_forwards_the_manifest() {
    use crate::project::editable::EditableProject;

    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), TOML_REQUIRED);
    let wrapped = EditableProject::new(
        typed_path::Utf8UnixPathBuf::from("lib"),
        toml_project(dir.path()),
    );
    assert_eq!(
        wrapped.manifest().expect("a manifest").name.as_str(),
        "widgets"
    );
}

// ----- which manifest a project without one is missing -----

#[test]
fn a_directory_with_no_manifest_at_all_names_all_three() {
    let dir = tempdir().expect("a temporary directory");
    let error = toml_project(dir.path())
        .checksum_canonical_variant()
        .expect_err("no manifest");
    assert_eq!(
        error.to_string(),
        "project is missing `model.toml`, or `.project.json` and/or `.meta.json`"
    );
}

/// The reason the message is not simply reworded for everyone: a json
/// project that has one of its two documents is not missing a
/// `model.toml`, and saying so would send the reader to write the wrong
/// file.
#[test]
fn a_json_project_missing_one_file_is_unchanged() {
    let dir = tempdir().expect("a temporary directory");
    write_meta_json(
        dir.path(),
        r#"{"index":{},"created":"1970-01-01T00:00:00Z"}"#,
    );
    let error = toml_project(dir.path())
        .checksum_canonical_variant()
        .expect_err("no `.project.json`");
    assert_eq!(
        error.to_string(),
        "project is missing `.project.json` and/or `.meta.json` files"
    );
}

// --- storing a manifest -----------------------------------------------

const MANIFEST: &str = "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\n";

fn document(text: &str) -> crate::manifest::edit::DocumentMut {
    text.parse().expect("a valid manifest document")
}

#[test]
fn a_local_project_writes_a_model_toml() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let mut project = LocalSrcProject::new_access(dir.path(), None);

    project.put_manifest_document(&document(MANIFEST), false)?;

    assert_eq!(
        std::fs::read_to_string(dir.path().join("model.toml"))?,
        MANIFEST
    );
    Ok(())
}

#[test]
fn writing_a_manifest_that_is_there_is_refused_without_overwrite()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_model_toml(dir.path(), MANIFEST);
    let mut project = LocalSrcProject::new_access(dir.path(), None);

    let error = project
        .put_manifest_document(&document(MANIFEST), false)
        .expect_err("the manifest is already there");

    assert!(error.to_string().contains("`model.toml` already exists"));
    Ok(())
}

#[test]
fn writing_a_manifest_with_overwrite_replaces_it() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_model_toml(dir.path(), MANIFEST);
    let mut project = LocalSrcProject::new_access(dir.path(), None);
    let replacement = MANIFEST.replace("1.0.0", "2.0.0");

    project.put_manifest_document(&document(&replacement), true)?;

    assert_eq!(
        std::fs::read_to_string(dir.path().join("model.toml"))?,
        replacement
    );
    Ok(())
}

#[test]
fn a_json_project_refuses_a_manifest_write() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"widgets","version":"1.0.0"}"#);
    let mut project = LocalSrcProject::new_access(dir.path(), None);

    let error = project
        .put_manifest_document(&document(MANIFEST), false)
        .expect_err("a json project cannot take a manifest");

    // The mirror of the message a json write into a toml project gets.
    assert!(error.to_string().contains("is a `.project.json` project"));
    assert!(error.to_string().contains("sysand migrate"));
    Ok(())
}

#[test]
fn a_manifest_written_to_a_local_project_is_read_back_as_toml()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let mut project = LocalSrcProject::new_access(dir.path(), None);
    project.put_manifest_document(&document(MANIFEST), false)?;

    assert_eq!(project.format()?, ProjectFormat::Toml);
    assert_eq!(
        project.manifest().expect("a manifest").name.as_str(),
        "widgets"
    );
    Ok(())
}

#[test]
fn a_manifest_overwritten_after_a_read_is_read_again() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_model_toml(dir.path(), MANIFEST);
    let mut project = LocalSrcProject::new_access(dir.path(), None);

    // Fills the cached view *before* the write, which is the only order
    // in which a stale cache can be observed.
    let (info, _) = project.get_project()?;
    assert_eq!(info.expect("info").version, "1.0.0");

    project.put_manifest_document(&document(&MANIFEST.replace("1.0.0", "2.0.0")), true)?;

    let (info, _) = project.get_project()?;
    assert_eq!(info.expect("info").version, "2.0.0");
    Ok(())
}

#[test]
fn a_local_project_hands_back_its_manifest_document() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let commented = "manifest-version = 1\n\n[project]\n# who publishes it\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\n";
    write_model_toml(dir.path(), commented);
    let project = LocalSrcProject::new_access(dir.path(), None);

    let document = project.manifest_document().expect("a document");
    assert_eq!(document.to_string(), commented);
    Ok(())
}

#[test]
fn a_json_project_hands_back_no_manifest_document() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"widgets","version":"1.0.0"}"#);
    let project = LocalSrcProject::new_access(dir.path(), None);

    assert!(project.manifest_document().is_err());
    Ok(())
}
