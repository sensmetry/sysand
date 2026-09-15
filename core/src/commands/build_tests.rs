// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use super::{
    KParBuildError, KparCompressionMethod, do_build_kpar, license_file_stems,
    read_optional_project_file,
};
use crate::project::utils::FsIoError;

#[test]
fn returns_none_when_project_root_is_none() {
    let result = read_optional_project_file(None, "README.md", "readme").unwrap();
    assert!(result.is_none());
}

#[test]
fn returns_none_when_file_does_not_exist() {
    let tmp = tempdir().unwrap();
    let result = read_optional_project_file(Some(tmp.path()), "README.md", "readme").unwrap();
    assert!(result.is_none());
}

#[test]
fn returns_content_when_file_exists() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("CHANGELOG.md"), b"# Changelog\n- entry\n").unwrap();

    let result = read_optional_project_file(Some(tmp.path()), "CHANGELOG.md", "changelog").unwrap();
    assert_eq!(result.as_deref(), Some("# Changelog\n- entry\n"));
}

#[test]
fn surfaces_non_not_found_io_errors() {
    let tmp = tempdir().unwrap();
    // A directory at the target path makes `read_to_string` fail with a
    // non-`NotFound` error, which the helper must propagate.
    std::fs::create_dir(tmp.path().join("README.md")).unwrap();

    let err = read_optional_project_file(Some(tmp.path()), "README.md", "readme").unwrap_err();
    match err {
        FsIoError::ReadFile(path, _) => assert_eq!(path, tmp.path().join("README.md")),
        other => panic!("expected FsIoError::ReadFile, got {other:?}"),
    }
}

fn stems(expr: &str) -> Vec<String> {
    license_file_stems(&spdx::Expression::parse(expr).unwrap())
        .into_iter()
        .collect()
}

#[test]
fn license_stems_single() {
    assert_eq!(stems("MIT"), vec!["MIT".to_owned()]);
}

#[test]
fn license_stems_compound_or() {
    assert_eq!(
        stems("MIT OR Apache-2.0"),
        vec!["MIT".to_owned(), "Apache-2.0".to_owned()]
    );
}

#[test]
fn license_stems_compound_and() {
    assert_eq!(
        stems("MIT AND BSD-2-Clause"),
        vec!["MIT".to_owned(), "BSD-2-Clause".to_owned()]
    );
}

#[test]
fn license_stems_with_exception() {
    assert_eq!(
        stems("GPL-2.0-only WITH Classpath-exception-2.0"),
        vec![
            "GPL-2.0-only".to_owned(),
            "Classpath-exception-2.0".to_owned(),
        ]
    );
}

#[test]
fn license_stems_license_ref() {
    assert_eq!(
        stems("LicenseRef-MyCustom"),
        vec!["LicenseRef-MyCustom".to_owned()]
    );
}

#[test]
fn license_stems_or_later_strips_plus() {
    // `MIT+` shares its license file with `MIT` per REUSE conventions —
    // the `+` does not appear in the bundled filename.
    assert_eq!(stems("MIT+"), vec!["MIT".to_owned()]);
}

#[test]
fn license_stems_deduplicates() {
    assert_eq!(stems("MIT AND MIT"), vec!["MIT".to_owned()]);
}

// ------------------------------- building a `model.toml` project

use crate::manifest::kpar_view::VIEW_CREATED;
use crate::model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw};
use crate::project::local_src::LocalSrcProject;

/// A `model.toml` project on disk, with `body` appended to the
/// `[project]` table, and one source file.
fn toml_project(body: &str) -> Result<camino_tempfile::Utf8TempDir, Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    std::fs::write(
        dir.path().join("model.toml"),
        format!(
            "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
             version = \"1.0.0\"\nsources = [\"a.sysml\"]\n{body}"
        ),
    )?;
    std::fs::write(dir.path().join("a.sysml"), b"package A;\n")?;
    Ok(dir)
}

/// Build it, and read the pair back out of the archive.
fn built(
    dir: &camino_tempfile::Utf8TempDir,
    allow_path_usage: bool,
) -> Result<
    (InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw),
    KParBuildError<crate::project::local_src::LocalSrcError>,
> {
    let project = LocalSrcProject::new_access(dir.path(), None);
    let path = dir.path().join("out.kpar");
    let kpar = do_build_kpar(
        &project,
        &path,
        KparCompressionMethod::default(),
        true,
        allow_path_usage,
    )?;
    let (info, meta) = crate::project::ProjectRead::get_project(&kpar).expect("the pair reads");
    Ok((info.expect("an info"), meta.expect("a meta")))
}

#[test]
fn a_toml_project_builds() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project("")?;
    let (info, _) = built(&dir, false)?;
    assert_eq!(info.name, "widgets");
    assert_eq!(info.version, "1.0.0");
    Ok(())
}

/// model-toml.md "TOML to KPAR": "`created` is the time of the build (so
/// two builds of identical sources are not byte-identical)". The view
/// records the epoch on purpose, and an archive must not.
#[test]
fn a_toml_build_records_the_build_time() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project("")?;
    let (_, meta) = built(&dir, false)?;
    let created: jiff::Timestamp = meta.created.parse()?;
    assert_ne!(
        created, VIEW_CREATED,
        "the view's placeholder reached the archive"
    );
    Ok(())
}

#[test]
fn a_toml_index_usage_becomes_a_purl() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\nversion = \"^1.2\"\n",
    )?;
    let (info, _) = built(&dir, false)?;
    assert_eq!(info.usage.len(), 1);
    assert!(
        format!("{:?}", info.usage[0]).contains("pkg:sysand/acme/dep"),
        "{:?}",
        info.usage[0]
    );
    Ok(())
}

#[test]
fn a_toml_url_usage_is_kept() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\n\
         url = \"https://example.com/dep.kpar\"\n",
    )?;
    let (info, _) = built(&dir, false)?;
    assert!(
        format!("{:?}", info.usage[0]).contains("https://example.com/dep.kpar"),
        "{:?}",
        info.usage[0]
    );
    Ok(())
}

/// "with it, local usages are emitted as the typed usages `Directory`
/// (source path) and `KparPath` (kpar path), which preserve the path as
/// written" -- which is what the view already produces, so no target is
/// read.
#[test]
fn a_local_usage_builds_under_allow_path_usage() -> Result<(), Box<dyn std::error::Error>> {
    // The target does not exist, and is named *inside* the fixture, so
    // this also pins that the flag skips the read entirely.
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"no-such-dep\"\n",
    )?;
    let (info, _) = built(&dir, true)?;
    let rendered = format!("{:?}", info.usage[0]);
    assert!(rendered.contains("Directory"), "{rendered}");
    assert!(rendered.contains("no-such-dep"), "{rendered}");
    Ok(())
}

/// model-toml.md "Standard library": the IRI comes from the metadata
/// Sysand embeds, "provided by the tooling".
#[test]
fn a_stdlib_index_usage_becomes_its_embedded_iri() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"OMG\"\nname = \"Kernel Semantic Library\"\n\
         version = \"^1.0.0\"\n",
    )?;
    let (info, _) = built(&dir, false)?;
    assert_eq!(
        resource(&info.usage[0]).0,
        "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar"
    );
    Ok(())
}

/// "with its version constraint" -- as declared, not as the embedded
/// release spells its version.
#[test]
fn a_stdlib_index_usage_keeps_its_declared_constraint() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"OMG\"\nname = \"Kernel Semantic Library\"\n\
         version = \"^1\"\n",
    )?;
    let (info, _) = built(&dir, false)?;
    assert_eq!(resource(&info.usage[0]).1, Some("^1".to_owned()));
    Ok(())
}

/// The stem is not derivable from the name: this one is published at
/// `Quantities-and-Units-Domain-Library.kpar`, and "Domain" appears in
/// the IRI and not in the name. So the resource is the map key.
#[test]
fn a_stdlib_name_whose_iri_differs_is_still_found() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"OMG\"\n\
         name = \"SysML Quantities and Units Library\"\nversion = \"^2.0.0\"\n",
    )?;
    let (info, _) = built(&dir, false)?;
    assert_eq!(
        resource(&info.usage[0]).0,
        "https://www.omg.org/spec/SysML/20250201/Quantities-and-Units-Domain-Library.kpar"
    );
    Ok(())
}

/// "A usage is a standard-library usage iff its publisher normalizes to
/// `omg`", and the name is matched normalized too.
#[test]
fn a_stdlib_usage_matches_case_insensitively() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"omg\"\nname = \"kernel semantic library\"\n\
         version = \"^1.0.0\"\n",
    )?;
    let (info, _) = built(&dir, false)?;
    assert_eq!(
        resource(&info.usage[0]).0,
        "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar"
    );
    Ok(())
}

/// The flag is about path usages: an index usage is rewritten either
/// way.
#[test]
fn a_stdlib_index_usage_is_rewritten_under_the_flag_too() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"OMG\"\nname = \"Kernel Semantic Library\"\n\
         version = \"^1.0.0\"\n",
    )?;
    let (info, _) = built(&dir, true)?;
    assert_eq!(
        resource(&info.usage[0]).0,
        "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar"
    );
    Ok(())
}

/// A standard-library usage no embedded release satisfies, reached with
/// a name that matches.
#[test]
fn an_unsatisfiable_stdlib_constraint_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"OMG\"\nname = \"Kernel Semantic Library\"\n\
         version = \"^9\"\n",
    )?;
    let error = built(&dir, false).expect_err("no embedded release satisfies `^9`");
    assert!(matches!(error, KParBuildError::TomlStdlibUnmatched { .. }));
    let text = error.to_string();
    assert!(text.contains("^9"), "{text}");
    assert!(text.contains("1.0.0"), "{text}");
    Ok(())
}

/// And with a name that does not. There is no embedded version to
/// quote, so the parenthesis says so instead.
#[test]
fn an_unknown_stdlib_name_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"OMG\"\nname = \"Not A Library\"\n\
         version = \"^1\"\n",
    )?;
    let error = built(&dir, false).expect_err("no such library is embedded");
    assert!(matches!(error, KParBuildError::TomlStdlibUnmatched { .. }));
    assert!(
        error.to_string().contains("no release of that name"),
        "{error}"
    );
    Ok(())
}

/// Only an `omg` publisher takes the standard-library path.
#[test]
fn a_non_omg_index_usage_is_untouched() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\nversion = \"^1\"\n",
    )?;
    let (info, _) = built(&dir, false)?;
    assert_eq!(resource(&info.usage[0]).0, "pkg:sysand/acme/dep");
    Ok(())
}

/// The gate: a json project with a typed local usage builds under
/// `--allow-path-usage`, and the toml refusals must not reach it.
#[test]
fn a_json_project_with_a_typed_local_usage_builds_under_the_flag()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    std::fs::write(
        dir.path().join(".project.json"),
        br#"{"name": "widgets", "publisher": "acme", "version": "1.0.0",
             "usage": [{"dir": "../dep", "publisher": "acme", "name": "dep"}]}"#,
    )?;
    std::fs::write(
        dir.path().join(".meta.json"),
        br#"{"created": "2001-02-03T04:05:06Z", "index": {}, "checksum": {}}"#,
    )?;
    built(&dir, true)?;
    Ok(())
}

/// Through `do_build_workspace_kpars`: a `model.toml` member built from
/// a workspace root records the build time too, not the view's epoch.
#[test]
fn a_workspace_of_toml_members_records_the_build_time() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    std::fs::write(
        dir.path().join(".workspace.json"),
        br#"{"projects": [{"path": "app", "iris": []}]}"#,
    )?;
    std::fs::create_dir_all(dir.path().join("app"))?;
    std::fs::write(
        dir.path().join("app/model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"alpha\"\n\
         version = \"1.0.0\"\nsources = [\"a.sysml\"]\n",
    )?;
    std::fs::write(dir.path().join("app/a.sysml"), b"package A;\n")?;

    let workspace = crate::workspace::Workspace::new(dir.path().to_owned())?;
    let out = dir.path().join("output");
    std::fs::create_dir_all(&out)?;
    let built = super::do_build_workspace_kpars(
        &workspace,
        &out,
        KparCompressionMethod::default(),
        true,
        false,
    )?;
    let (_, meta) = crate::project::ProjectRead::get_project(&built[0]).expect("the pair reads");
    let created: jiff::Timestamp = meta.expect("a meta").created.parse()?;
    assert_ne!(created, VIEW_CREATED);
    Ok(())
}

/// model-toml.md "TOML to KPAR": "`publisher` goes into
/// `project.publisher`, unless the project carries `stdlib`", so that a
/// locally built standard-library KPAR matches the published one.
///
/// `ProjectManifest::resolve` accepts `stdlib` only for a publisher
/// normalizing to `omg` under a known OMG metamodel, so the fixture
/// needs a real metamodel URL.
#[test]
fn a_stdlib_project_omits_its_publisher() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"omg\"\nname = \"kerml\"\n\
         version = \"1.0.0\"\nsources = [\"a.sysml\"]\nstdlib = \"KerML\"\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n",
    )?;
    std::fs::write(dir.path().join("a.sysml"), b"package A;\n")?;

    let (info, _) = built(&dir, false)?;
    assert_eq!(
        info.publisher, None,
        "a stdlib archive carries no publisher"
    );
    assert_eq!(info.name, "kerml");
    Ok(())
}

#[test]
fn a_non_stdlib_project_keeps_its_publisher() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project("")?;
    let (info, _) = built(&dir, false)?;
    assert_eq!(info.publisher.as_deref(), Some("acme"));
    Ok(())
}

// ------------------------------------ a local usage becomes a PURL

/// A legacy json project at `at`, inside the fixture.
fn json_target(dir: &camino_tempfile::Utf8TempDir, at: &str, version: &str) {
    let path = dir.path().join(at);
    std::fs::create_dir_all(&path).expect("the target directory");
    std::fs::write(
        path.join(".project.json"),
        format!(r#"{{"name": "dep", "publisher": "acme", "version": "{version}", "usage": []}}"#),
    )
    .expect("a target manifest");
    std::fs::write(
        path.join(".meta.json"),
        br#"{"created": "2001-02-03T04:05:06Z", "index": {}, "checksum": {}}"#,
    )
    .expect("a target metadata");
}

/// A `model.toml` project at `at`, with `body` appended to `[project]`
/// and its one source file written -- a target whose `sources` named a
/// missing file would fail the *dependent's* build if the version were
/// read through `get_info`.
fn toml_target(dir: &camino_tempfile::Utf8TempDir, at: &str, version: &str, body: &str) {
    let path = dir.path().join(at);
    std::fs::create_dir_all(&path).expect("the target directory");
    std::fs::write(
        path.join("model.toml"),
        format!(
            "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"dep\"\n\
             version = \"{version}\"\nsources = [\"b.sysml\"]\n{body}"
        ),
    )
    .expect("a target manifest");
    std::fs::write(path.join("b.sysml"), b"package B;\n").expect("a target source");
}

/// A `model.toml` standard-library project at `at`: publisher `omg`, a
/// `stdlib` stem and an inline OMG metamodel.
///
/// Inline and not `metamodel.workspace`, which is what model-toml.md's
/// own example uses: the target is read standalone, with no workspace
/// to inherit a metamodel from.
fn stdlib_target(dir: &camino_tempfile::Utf8TempDir, at: &str) {
    let path = dir.path().join(at);
    std::fs::create_dir_all(&path).expect("the target directory");
    std::fs::write(
        path.join("model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"omg\"\nname = \"dep\"\n\
         version = \"1.0.0\"\nsources = [\"b.kerml\"]\nstdlib = \"Semantic-Library\"\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n",
    )
    .expect("a target manifest");
    std::fs::write(path.join("b.kerml"), b"package B;\n").expect("a target source");
}

const DIR_USAGE: &str =
    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"dep\"\n";

fn resource(raw: &crate::model::InterchangeProjectUsageRaw) -> (String, Option<String>) {
    match raw {
        crate::model::InterchangeProjectUsageRaw::Resource {
            resource,
            version_constraint,
        } => (resource.clone(), version_constraint.clone()),
        other => panic!("expected a resource usage, got `{other:?}`"),
    }
}

#[test]
fn a_dir_usage_becomes_a_caret_purl() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(DIR_USAGE)?;
    json_target(&dir, "dep", "1.2.3");
    let (info, _) = built(&dir, false)?;
    assert_eq!(
        resource(&info.usage[0]),
        ("pkg:sysand/acme/dep".to_owned(), Some("^1.2.3".to_owned()))
    );
    Ok(())
}

#[test]
fn a_dir_usage_reads_a_toml_target() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(DIR_USAGE)?;
    toml_target(&dir, "dep", "4.5.6", "");
    let (info, _) = built(&dir, false)?;
    assert_eq!(resource(&info.usage[0]).1, Some("^4.5.6".to_owned()));
    Ok(())
}

/// The PURL form normalizes; the typed form does not. That asymmetry is
/// the specification's.
#[test]
fn a_dir_usage_normalizes_the_purl() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"ACME\"\nname = \"Wid Gets\"\ndir = \"dep\"\n",
    )?;
    json_target(&dir, "dep", "1.0.0");
    let (info, _) = built(&dir, false)?;
    assert_eq!(resource(&info.usage[0]).0, "pkg:sysand/acme/wid-gets");
    Ok(())
}

#[test]
fn a_missing_target_defines_no_project() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(DIR_USAGE)?;
    let error = built(&dir, false).expect_err("there is no `dep`");
    assert!(matches!(error, KParBuildError::TomlTargetMissing { .. }));
    let text = error.to_string();
    assert!(text.contains("acme/dep"), "{text}");
    assert!(text.contains("defines no project"), "{text}");
    Ok(())
}

#[test]
fn an_empty_target_defines_no_project() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(DIR_USAGE)?;
    std::fs::create_dir_all(dir.path().join("dep"))?;
    let error = built(&dir, false).expect_err("`dep` holds no manifest");
    assert!(matches!(error, KParBuildError::TomlTargetMissing { .. }));
    Ok(())
}

/// A target whose version is not semver, refused with a hint. Reachable
/// only for a json target.
#[test]
fn a_non_semver_target_is_refused_with_a_hint() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(DIR_USAGE)?;
    json_target(&dir, "dep", "2025-02-01");
    let error = built(&dir, false).expect_err("`2025-02-01` is not semver");
    assert!(matches!(error, KParBuildError::TomlTargetVersion { .. }));
    let text = error.to_string();
    assert!(text.contains("2025-02-01"), "{text}");
    assert!(text.contains("--allow-path-usage"), "{text}");
    Ok(())
}

/// A target that is there and does not read is neither
/// `TomlTargetVersion` nor `TomlTargetMissing`, and cannot travel in
/// `ProjectRead`, which carries the error type of the project being
/// *built*.
#[test]
fn a_target_that_does_not_read_is_its_own_error() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(DIR_USAGE)?;
    json_target(&dir, "dep", "1.0.0");
    toml_target(&dir, "dep", "1.0.0", "");
    let error = built(&dir, false).expect_err("`dep` holds both manifests");
    assert!(matches!(error, KParBuildError::TomlTargetRead { .. }));
    Ok(())
}

/// model-toml.md "Standard library": "a usage whose target carries
/// `stdlib` is written as `resource = "<target's metamodel>/<target's
/// stdlib>.kpar"` -- the metamodel supplies the prefix and release date
/// -- with its version constraint, instead of as a PURL".
#[test]
fn a_local_stdlib_target_becomes_an_omg_iri() -> Result<(), Box<dyn std::error::Error>> {
    let dir =
        toml_project("\n[[inline-usages]]\npublisher = \"omg\"\nname = \"dep\"\ndir = \"dep\"\n")?;
    stdlib_target(&dir, "dep");

    let (info, _) = built(&dir, false)?;
    assert_eq!(
        resource(&info.usage[0]),
        (
            "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar".to_owned(),
            // Only the resource changes: "instead of the PURL" names one
            // substitution, so the constraint is still rule 2's.
            Some("^1.0.0".to_owned())
        )
    );
    Ok(())
}

/// "Under `--allow-path-usage` a local usage stays a typed path usage,
/// stdlib target or not".
#[test]
fn a_local_stdlib_target_still_stays_typed_under_the_flag() -> Result<(), Box<dyn std::error::Error>>
{
    let dir =
        toml_project("\n[[inline-usages]]\npublisher = \"omg\"\nname = \"dep\"\ndir = \"dep\"\n")?;
    stdlib_target(&dir, "dep");

    let (info, _) = built(&dir, true)?;
    assert!(format!("{:?}", info.usage[0]).contains("Directory"));
    Ok(())
}

/// The typed list and the raw list are the same usages in the same
/// order, so the rewrite may patch positionally.
#[test]
fn the_typed_and_raw_usage_lists_correspond() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"idx\"\nversion = \"^2\"\n\
         \n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"dep\"\n\
         \n[[inline-usages]]\npublisher = \"acme\"\nname = \"rem\"\nurl = \"https://example.com/r.kpar\"\n",
    )?;
    json_target(&dir, "dep", "1.2.3");
    let (info, _) = built(&dir, false)?;
    assert_eq!(info.usage.len(), 3);
    // The index usage keeps its own constraint: only the local one is
    // rewritten, and it is rewritten in its own slot.
    assert_eq!(
        resource(&info.usage[0]),
        ("pkg:sysand/acme/idx".to_owned(), Some("^2".to_owned()))
    );
    assert_eq!(
        resource(&info.usage[1]),
        ("pkg:sysand/acme/dep".to_owned(), Some("^1.2.3".to_owned()))
    );
    assert_eq!(resource(&info.usage[2]).0, "https://example.com/r.kpar");
    Ok(())
}

/// A `kpar-path` target is a *file*, and its version comes from the
/// archive's own `.project.json`.
///
/// The target here is built from a **stdlib** project, so this also
/// pins that rule 3 cannot reach an archive: a KPAR carries no
/// `model.toml` (one at its root is `TomlInKpar`), so a publisher-less
/// standard-library archive still gets rule 2's PURL.
#[test]
fn a_kpar_path_usage_becomes_a_caret_purl() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\nkpar-path = \"dep.kpar\"\n",
    )?;
    // Build the target archive first.
    let target_dir = tempdir()?;
    std::fs::write(
        target_dir.path().join("model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"omg\"\nname = \"dep\"\n\
         version = \"7.8.9\"\nsources = [\"a.kerml\"]\nstdlib = \"Semantic-Library\"\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n",
    )?;
    std::fs::write(target_dir.path().join("a.kerml"), b"package A;\n")?;
    let target = LocalSrcProject::new_access(target_dir.path(), None);
    do_build_kpar(
        &target,
        dir.path().join("dep.kpar"),
        KparCompressionMethod::default(),
        true,
        false,
    )?;

    let (info, _) = built(&dir, false)?;
    assert_eq!(
        resource(&info.usage[0]),
        ("pkg:sysand/acme/dep".to_owned(), Some("^7.8.9".to_owned()))
    );
    Ok(())
}

/// A `kpar-path` entry naming a directory, and a `dir` entry naming a
/// file, both define no project. Neither may surface as an IO error:
/// joining `model.toml` onto a file gives `ENOTDIR`, not `NotFound`.
#[test]
fn a_target_of_the_wrong_kind_defines_no_project() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\nkpar-path = \"dep\"\n",
    )?;
    std::fs::create_dir_all(dir.path().join("dep"))?;
    let error = built(&dir, false).expect_err("a directory is not an archive");
    assert!(
        matches!(error, KParBuildError::TomlTargetMissing { .. }),
        "{error}"
    );

    let other = toml_project(DIR_USAGE)?;
    std::fs::write(other.path().join("dep"), b"not a directory\n")?;
    let error = built(&other, false).expect_err("a file is not a project directory");
    assert!(
        matches!(error, KParBuildError::TomlTargetMissing { .. }),
        "{error}"
    );
    Ok(())
}
