// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use std::assert_matches;

use super::{LocalSrcError, LocalSrcProject, ProjectFormat, detect_format};
use crate::model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw};
use crate::project::{FormatError, ProjectMut as _, ProjectRead as _};

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
    write_meta_json(
        dir.path(),
        r#"{"index":{},"created":"1970-01-01T00:00:00Z"}"#,
    );

    let project = LocalSrcProject::new_access(dir.path(), None).get_project()?;
    assert_eq!(project.expect("a project").name(), "p");
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
    write_model_toml(dir.path(), "manifest-version = \"1.0\"\n");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Toml
    );
}

#[test]
fn a_directory_with_both_is_ambiguous() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = \"1.0\"\n");
    write_project_json(dir.path(), "{}");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Both
    );
}

/// A directory with both a `model.toml` and json config is unclear: it
/// could be a non-zipped KPAR or a development project.
#[test]
fn an_ambiguous_directory_is_refused_when_it_is_read() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = \"1.0\"\n");
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
    write_model_toml(dir.path(), "manifest-version = \"1.0\"\n");
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

/// The keys `[project]` requires: the three every metamodel needs, the
/// `metamodel` itself, and the `sources` an OMG-metamodel project lists.
const TOML_REQUIRED: &str = "\
manifest-version = \"1.0\"

[project]
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
";

fn toml_project(dir: &camino::Utf8Path) -> LocalSrcProject {
    LocalSrcProject::new_access(dir, None)
}

/// The manifest of a project, which must be a `model.toml` one.
fn toml_manifest(project: &LocalSrcProject) -> Box<crate::manifest::project::ResolvedProject> {
    match project.get_project().expect("readable") {
        Some(crate::project::ProjectDefinition::Toml(manifest)) => manifest,
        other => panic!("not a `model.toml` project: {other:?}"),
    }
}

#[test]
fn a_model_toml_project_reads_as_its_manifest() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        "manifest-version = \"1.0\"\n\n\
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

    let manifest = toml_manifest(&toml_project(dir.path()));
    assert_eq!(manifest.name.as_str(), "widgets");
    assert_eq!(manifest.publisher.as_str(), "acme");
    assert_eq!(manifest.version.to_string(), "1.2.3");
    assert_eq!(manifest.description.as_deref(), Some("A description."));
    assert_eq!(
        manifest
            .license
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("MIT")
    );
    assert_eq!(
        manifest.metamodel.to_string(),
        "https://www.omg.org/spec/SysML/20250201"
    );
    assert_eq!(
        manifest
            .sources()
            .expect("an OMG project")
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>(),
        ["a.sysml"]
    );
}

/// Reading a `model.toml` project reads the manifest alone: a source
/// that is missing or does not parse is found when the sources are used.
#[test]
fn reading_a_model_toml_project_reads_no_source() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        &TOML_REQUIRED.replace("sources = []", "sources = [\"gone.sysml\", \"a.sysml\"]"),
    );
    std::fs::write(dir.path().join("a.sysml"), "package Foo { /*").expect("write a.sysml");

    let manifest = toml_manifest(&toml_project(dir.path()));
    assert_eq!(
        manifest.sources().expect("an OMG project").iter().count(),
        2
    );
}

/// A `model.toml` project has no project checksum, so it cannot be a
/// source pinned by one.
#[test]
fn a_model_toml_project_has_no_project_checksum() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), TOML_REQUIRED);
    let project = LocalSrcProject::new_access(dir.path(), Some("lib".into()));

    assert_matches!(
        project.checksum_canonical_variant(),
        Err(LocalSrcError::Format {
            dir: _,
            problem: FormatError::TomlHasNoChecksum
        })
    );
    assert_matches!(
        project.sources(&crate::context::ProjectContext::default()),
        Err(LocalSrcError::Format {
            dir: _,
            problem: FormatError::TomlHasNoChecksum
        })
    );
}

#[test]
#[should_panic(expected = "which has no project hash")]
fn hashing_a_model_toml_project_panics() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), TOML_REQUIRED);
    toml_project(dir.path())
        .checksum_canonical_hex()
        .expect("never returns");
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
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\n",
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
        "manifest-version = \"1.0\"\n\n[project]\nsources = []\n",
    );
    let error = read_error(dir.path());
    for key in ["publisher", "name", "version"] {
        assert!(
            error.contains(&format!("missing key `{key}` in `[project]`")),
            "`{key}` is not reported in: {error}"
        );
    }
}

/// A project read on its own resolves against no table, so a member's
/// manifest read that way says exactly that. `new_member` is the way to
/// read one as a member.
#[test]
fn a_model_toml_that_inherits_outside_a_workspace_says_so() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        "manifest-version = \"1.0\"\n\n\
         [project]\n\
         name = \"widgets\"\n\
         version = \"1.2.3\"\n\
         publisher.workspace = true\n\
         sources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );
    assert!(
        read_error(dir.path()).contains("not a member of any workspace"),
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
        LocalSrcError::Format {
            dir: _,
            problem: FormatError::JsonWriteToToml(_)
        }
    );
}

/// `do_include` reads the metadata (supplied by the view) and writes it
/// back; the guard stops it leaving a `.meta.json` beside the `model.toml`.
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

// ----- the manifest -----

/// A `.project.json` has no room for some of what a `model.toml` says
/// (`urls.repository` and `urls.documentation`, for one), and the manifest
/// carries it.
#[test]
fn a_manifest_carries_what_a_project_json_has_no_room_for() {
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

    let manifest = toml_manifest(&toml_project(dir.path()));
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
}

#[test]
fn a_json_project_reads_as_its_two_documents() {
    let dir = tempdir().expect("a temporary directory");
    write_project_json(
        dir.path(),
        r#"{"name":"widgets","publisher":"acme","version":"1.2.3"}"#,
    );
    write_meta_json(
        dir.path(),
        r#"{"index":{},"created":"1970-01-01T00:00:00Z"}"#,
    );
    assert_matches!(
        toml_project(dir.path()).get_project(),
        Ok(Some(crate::project::ProjectDefinition::Json {
            info: _,
            meta: _
        }))
    );
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
    assert_matches!(
        wrapped.get_project(),
        Ok(Some(crate::project::ProjectDefinition::Toml(manifest)))
            if manifest.name.as_str() == "widgets"
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
        "project is missing `model.toml`, or `.project.json` and `.meta.json`"
    );
}

/// A json project that has one of its two documents is not reported as
/// missing a `model.toml`.
#[test]
fn a_json_project_missing_one_file_names_it() {
    let dir = tempdir().expect("a temporary directory");
    write_meta_json(
        dir.path(),
        r#"{"index":{},"created":"1970-01-01T00:00:00Z"}"#,
    );
    let error = toml_project(dir.path())
        .get_project()
        .expect_err("no `.project.json`");
    assert_eq!(
        error.to_string(),
        format!("`{}` is missing `.project.json`", dir.path())
    );
}

// --- storing a manifest -----------------------------------------------

const MANIFEST: &str = "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n";

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
    let replacement = MANIFEST.replace("\nversion = \"1.0.0\"", "\nversion = \"2.0.0\"");

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
    assert_eq!(toml_manifest(&project).name.as_str(), "widgets");
    Ok(())
}

#[test]
fn a_manifest_overwritten_after_a_read_is_read_again() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_model_toml(dir.path(), MANIFEST);
    let mut project = LocalSrcProject::new_access(dir.path(), None);

    assert_eq!(toml_manifest(&project).version.to_string(), "1.0.0");

    project.put_manifest_document(
        &document(&MANIFEST.replace("\nversion = \"1.0.0\"", "\nversion = \"2.0.0\"")),
        true,
    )?;

    assert_eq!(toml_manifest(&project).version.to_string(), "2.0.0");
    Ok(())
}

#[test]
fn a_local_project_hands_back_its_manifest_document() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let commented = "manifest-version = \"1.0\"\n\n[project]\n# who publishes it\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n";
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

    project
        .manifest_document()
        .expect_err("a json project has no manifest document");
    Ok(())
}

// ------------------- a project that knows its workspace

/// A table whose default group carries `version = "9.9.9"`.
fn versioned_table() -> std::sync::Arc<crate::manifest::workspace_table::WorkspaceTable> {
    std::sync::Arc::new(crate::manifest::workspace_table::WorkspaceTable {
        default_group: crate::manifest::workspace_table::GroupProperties {
            version: Some("9.9.9".parse().expect("a version")),
            ..Default::default()
        },
        ..Default::default()
    })
}

const INHERITS_VERSION: &str = "manifest-version = \"1.0\"\n\n\
     [project]\n\
     publisher = \"acme\"\n\
     name = \"widgets\"\n\
     version.workspace = true\n\
     sources = []\n\
     metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n";

/// A member's marker follows the table it was given.
#[test]
fn a_member_project_resolves_against_its_table() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), INHERITS_VERSION);

    let manifest = toml_manifest(&LocalSrcProject::new_member(
        dir.path(),
        None,
        versioned_table(),
    ));
    assert_eq!(manifest.version.to_string(), "9.9.9");
}

/// A value the project states itself wins over the group's (the fixture
/// values differ).
#[test]
fn a_member_project_prefers_its_own_values() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(
        dir.path(),
        "manifest-version = \"1.0\"\n\n\
         [project]\n\
         publisher = \"acme\"\n\
         name = \"widgets\"\n\
         version = \"1.2.3\"\n\
         sources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );

    let manifest = toml_manifest(&LocalSrcProject::new_member(
        dir.path(),
        None,
        versioned_table(),
    ));
    assert_eq!(manifest.version.to_string(), "1.2.3");
}

/// The extracted form of a Syside archive is a json project like any
/// other. Detection looks at which files are there, not at their shape.
#[test]
fn a_root_shaped_json_pair_is_detected_as_a_json_project() {
    let dir = tempdir().expect("a temporary directory");
    write_project_json(
        dir.path(),
        r#"{"name": "widgets", "publisher": "acme", "version": "1.0.0"}"#,
    );
    write_meta_json(
        dir.path(),
        r#"{"root": "main.sysml", "exports": [], "created": "2026-01-01T00:00:00Z",
            "metamodel": "https://www.sysand.com/spec/SysML/20250201"}"#,
    );
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Json
    );
}
