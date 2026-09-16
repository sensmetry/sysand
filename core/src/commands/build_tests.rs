// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use super::{
    KParBuildError, KparCompressionMethod, do_build_kpar, license_file_stems,
    read_optional_project_file,
};
use crate::{manifest::MODEL_TOML_FILE, project::utils::FsIoError};

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
    assert!(matches!(
        error,
        KParBuildError::TomlStdlibUnmatched {
            usage: _,
            constraint: _,
            embedded: _
        }
    ));
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
    assert!(matches!(
        error,
        KParBuildError::TomlStdlibUnmatched {
            usage: _,
            constraint: _,
            embedded: _
        }
    ));
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
        dir.path().join(MODEL_TOML_FILE),
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\n",
    )?;
    std::fs::create_dir_all(dir.path().join("app"))?;
    std::fs::write(
        dir.path().join("app/model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"alpha\"\n\
         version = \"1.0.0\"\nsources = [\"a.sysml\"]\n",
    )?;
    std::fs::write(dir.path().join("app/a.sysml"), b"package A;\n")?;

    let workspace = load_workspace(&dir);
    let out = dir.path().join("output");
    std::fs::create_dir_all(&out)?;
    let built = super::do_build_workspace_kpars(
        &workspace,
        &out,
        KparCompressionMethod::default(),
        true,
        false,
    )?;
    let (_, meta) =
        crate::project::ProjectRead::get_project(built.first().expect("one member was built"))
            .expect("the pair reads");
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
    assert!(matches!(
        error,
        KParBuildError::TomlTargetMissing { usage: _, path: _ }
    ));
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
    assert!(matches!(
        error,
        KParBuildError::TomlTargetMissing { usage: _, path: _ }
    ));
    Ok(())
}

/// A target whose version is not semver, refused with a hint. Reachable
/// only for a json target.
#[test]
fn a_non_semver_target_is_refused_with_a_hint() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(DIR_USAGE)?;
    json_target(&dir, "dep", "2025-02-01");
    let error = built(&dir, false).expect_err("`2025-02-01` is not semver");
    assert!(matches!(
        error,
        KParBuildError::TomlTargetVersion {
            usage: _,
            path: _,
            version: _
        }
    ));
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
    assert!(matches!(
        error,
        KParBuildError::TomlTargetRead {
            usage: _,
            path: _,
            reason: _
        }
    ));
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
        matches!(
            error,
            KParBuildError::TomlTargetMissing { usage: _, path: _ }
        ),
        "{error}"
    );

    let other = toml_project(DIR_USAGE)?;
    std::fs::write(other.path().join("dep"), b"not a directory\n")?;
    let error = built(&other, false).expect_err("a file is not a project directory");
    assert!(
        matches!(
            error,
            KParBuildError::TomlTargetMissing { usage: _, path: _ }
        ),
        "{error}"
    );
    Ok(())
}

// -------------------------------------- members build in `members` order

/// A valid project name for a usage of `dep`. The build runs with
/// `--allow-path-usage`, so no target is read and the name only has to
/// satisfy the manifest's own rules, which a name of fewer than three
/// characters does not.
fn usage_name(dep: &str) -> String {
    let last: String = dep
        .rsplit('/')
        .next()
        .unwrap_or(dep)
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    if last.len() < 3 {
        "selfdep".to_owned()
    } else {
        last
    }
}

fn workspace_root(dir: &camino_tempfile::Utf8TempDir, members: &[&str]) {
    let entries: Vec<String> = members.iter().map(|m| format!("\"{m}\"")).collect();
    std::fs::write(
        dir.path().join(MODEL_TOML_FILE),
        format!(
            "manifest-version = 1\n\n[workspace]\nmembers = [{}]\n",
            entries.join(", ")
        ),
    )
    .expect("a workspace file");
}

/// Loads the workspace with `MemberChecks::SkipStrayState`.
fn load_workspace(dir: &camino_tempfile::Utf8TempDir) -> crate::workspace::model_toml::Workspace {
    crate::workspace::model_toml::Workspace::load_with(
        dir.path(),
        crate::workspace::model_toml::MemberChecks::SkipStrayState,
    )
    .expect("the workspace loads")
}

/// A `model.toml` member. `stdlib` needs publisher `omg` and an OMG
/// metamodel, which `ProjectManifest::resolve` enforces.
fn toml_member(dir: &camino_tempfile::Utf8TempDir, at: &str, deps: &[&str], stdlib: bool) {
    let path = dir.path().join(at);
    std::fs::create_dir_all(&path).expect("the member directory");
    let head = if stdlib {
        format!(
            "manifest-version = 1\n\n[project]\npublisher = \"omg\"\nname = \"{at}\"\n\
             version = \"1.0.0\"\nsources = [\"m.kerml\"]\nstdlib = \"Semantic-Library\"\n\
             metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n"
        )
    } else {
        format!(
            "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"{at}\"\n\
             version = \"1.0.0\"\nsources = [\"m.kerml\"]\n"
        )
    };
    let mut manifest = head;
    for d in deps {
        let name = usage_name(d);
        manifest.push_str("\n[[inline-usages]]\npublisher = \"acme\"\nname = \"");
        manifest.push_str(&name);
        manifest.push_str("\"\ndir = \"");
        manifest.push_str(d);
        manifest.push_str("\"\n");
    }
    std::fs::write(path.join("model.toml"), manifest).expect("a member manifest");
    std::fs::write(path.join("m.kerml"), b"package M;\n").expect("a member source");
}

/// The names of the archives, in the order they were built, of a
/// workspace in which every member builds.
fn build_workspace(dir: &camino_tempfile::Utf8TempDir) -> Vec<String> {
    names(
        &build_workspace_result(dir)
            .unwrap_or_else(|failure| panic!("{failure}: {:?}", failure.failed)),
    )
}

fn build_workspace_result(
    dir: &camino_tempfile::Utf8TempDir,
) -> Result<Vec<super::LocalKParProjectRaw>, super::WorkspaceBuildFailure> {
    let workspace = load_workspace(dir);
    let out = dir.path().join("output");
    std::fs::create_dir_all(&out).expect("the output directory");
    super::do_build_workspace_kpars(
        &workspace,
        &out,
        KparCompressionMethod::default(),
        true,
        true,
    )
}

/// workspaces.md "Build output": every member is built, in `members`
/// order, whatever their `dir` usages of each other.
#[test]
fn members_build_in_members_order() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["gamma", "beta", "alpha"]);
    toml_member(&dir, "alpha", &[], false);
    toml_member(&dir, "beta", &["../alpha"], false);
    toml_member(&dir, "gamma", &["../beta"], false);
    assert_eq!(build_workspace(&dir), vec!["gamma", "beta", "alpha"]);
    Ok(())
}

/// "Nodes keep the order they were added in": with no edges, `members`
/// order is the answer.
#[test]
fn independent_members_keep_their_declared_order() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["gamma", "beta", "alpha"]);
    for at in ["alpha", "beta", "gamma"] {
        toml_member(&dir, at, &[], false);
    }
    assert_eq!(build_workspace(&dir), vec!["gamma", "beta", "alpha"]);
    Ok(())
}

/// "a project's usages routinely name projects outside it": such a
/// usage is not an edge, and the outsider is not built.
#[test]
fn a_usage_of_a_non_member_is_not_an_edge() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["gamma", "beta"]);
    toml_member(&dir, "outside", &[], false);
    toml_member(&dir, "beta", &["../outside"], false);
    toml_member(&dir, "gamma", &[], false);
    assert_eq!(build_workspace(&dir), vec!["gamma", "beta"]);
    Ok(())
}

// ------------------------------ a failed member does not stop the others

/// A member that loads and cannot be built: its manifest is valid, so
/// the workspace loads, and the source it lists is not there, so
/// `default_kpar_file_name` -- which reads the project -- is where it
/// fails, before the build proper.
///
/// Not an empty directory: `load_member` refuses one ("defines no
/// project"), which fails the whole load and makes the per-member
/// reporting this section tests unreachable.
fn broken_member(dir: &camino_tempfile::Utf8TempDir, at: &str) {
    let path = dir.path().join(at);
    std::fs::create_dir_all(&path).expect("the member directory");
    std::fs::write(
        path.join(MODEL_TOML_FILE),
        format!(
            "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"{at}\"\n\
             version = \"1.0.0\"\nsources = [\"missing.kerml\"]\n"
        ),
    )
    .expect("a member manifest");
}

/// The project names of `archives`.
fn names(archives: &[super::LocalKParProjectRaw]) -> Vec<String> {
    archives
        .iter()
        .map(|p| {
            crate::project::ProjectRead::name(p)
                .ok()
                .flatten()
                .unwrap_or_default()
        })
        .collect()
}

/// The failure of a workspace build that is expected to fail.
fn build_workspace_failure(dir: &camino_tempfile::Utf8TempDir) -> super::WorkspaceBuildFailure {
    build_workspace_result(dir).expect_err("a member fails")
}

fn failed_members(failure: &super::WorkspaceBuildFailure) -> Vec<&str> {
    failure
        .failed
        .iter()
        .map(|failed| failed.member.as_str())
        .collect()
}

#[test]
fn a_failed_member_does_not_stop_an_independent_one() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["alpha", "beta"]);
    broken_member(&dir, "alpha");
    toml_member(&dir, "beta", &[], false);

    let failure = build_workspace_failure(&dir);
    assert_eq!(failed_members(&failure), ["alpha"]);
    assert_eq!(names(&failure.built), ["beta"]);
    assert_eq!(
        failure.to_string(),
        "1 of 2 workspace members failed to build"
    );
    Ok(())
}

/// A build never reads another member's archive, so a member whose
/// dependency failed builds all the same.
#[test]
fn a_failed_member_does_not_stop_its_dependents() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["beta", "alpha"]);
    broken_member(&dir, "alpha");
    toml_member(&dir, "beta", &["../alpha"], false);

    let failure = build_workspace_failure(&dir);
    assert_eq!(failed_members(&failure), ["alpha"]);
    assert_eq!(names(&failure.built), ["beta"]);
    Ok(())
}

#[test]
fn a_workspace_that_builds_reports_every_member_built() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["gamma", "beta", "alpha"]);
    toml_member(&dir, "alpha", &[], false);
    toml_member(&dir, "beta", &["../alpha"], false);
    toml_member(&dir, "gamma", &["../beta"], false);

    assert_eq!(build_workspace(&dir).len(), 3);
    Ok(())
}

/// `default_kpar_file_name` is the call that fails for a member whose
/// project cannot be read. Outside the per-member arm it would stop the
/// workspace.
#[test]
fn a_member_that_cannot_be_read_fails_as_itself() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["alpha"]);
    broken_member(&dir, "alpha");

    let failure = build_workspace_failure(&dir);
    let [failed] = failure.failed.as_slice() else {
        panic!("one failure: {:?}", failure.failed);
    };
    assert_eq!(failed.member, "alpha");
    assert!(
        matches!(*failed.error, KParBuildError::ProjectRead(_)),
        "{}",
        failed.error
    );
    Ok(())
}

// --------------------------------------------------- the publish order

#[test]
fn publish_order_is_the_dependency_order() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["gamma", "beta", "alpha"]);
    toml_member(&dir, "alpha", &[], false);
    toml_member(&dir, "beta", &["../alpha"], false);
    toml_member(&dir, "gamma", &["../beta"], false);

    let workspace = load_workspace(&dir);
    let order = super::workspace_publish_order(&workspace).map_err(|e| e.to_string())?;

    let root = crate::project::utils::wrapfs::canonicalize(dir.path())?;
    assert_eq!(
        order,
        vec![
            ("alpha".into(), root.join("output/alpha-1.0.0.kpar")),
            ("beta".into(), root.join("output/beta-1.0.0.kpar")),
            ("gamma".into(), root.join("output/gamma-1.0.0.kpar")),
        ],
        "the full archive path matters: a member-directory `output/` is also under the root"
    );
    Ok(())
}

/// Found while mapping members to archives, before the caller uploads
/// anything -- and named, since `KParBuildError` carries no identity.
#[test]
fn publish_order_refuses_a_member_that_cannot_be_read() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["alpha", "beta"]);
    broken_member(&dir, "alpha");
    toml_member(&dir, "beta", &[], false);

    let workspace = load_workspace(&dir);
    let failure = super::workspace_publish_order(&workspace).expect_err("alpha cannot be read");
    assert_eq!(failure.member, "alpha");
    assert!(
        matches!(*failure.error, KParBuildError::ProjectRead(_)),
        "{failure}"
    );
    Ok(())
}

/// A member that inherits a key can be the *target* of another member's
/// `dir` usage.
///
/// `local_target_usage` must open the target with the workspace table:
/// opened without one, a marker on the target is unresolvable and the
/// usage fails with "not a member of any workspace" -- about a project
/// that is one.
#[test]
fn a_member_that_inherits_can_be_a_usage_target() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    workspace_root(&dir, &["app", "dep"]);
    std::fs::write(
        dir.path().join(MODEL_TOML_FILE),
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\", \"dep\"]\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    )?;
    for (at, extra) in [
        ("dep", ""),
        (
            "app",
            "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../dep\"\n",
        ),
    ] {
        let path = dir.path().join(at);
        std::fs::create_dir_all(&path)?;
        std::fs::write(
            path.join(MODEL_TOML_FILE),
            format!(
                "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"{at}\"\n\
                 version = \"1.0.0\"\nmetamodel.workspace = true\n\
                 sources = [\"m.kerml\"]\n{extra}"
            ),
        )?;
        std::fs::write(path.join("m.kerml"), b"package M;\n")?;
    }

    // `allow_path_usage: false`, as `sysand build` passes it: with it
    // true, a local usage stays typed and the target is never read at all.
    let workspace = load_workspace(&dir);
    let out = dir.path().join("output");
    std::fs::create_dir_all(&out)?;
    if let Err(failure) = super::do_build_workspace_kpars(
        &workspace,
        &out,
        KparCompressionMethod::default(),
        true,
        false,
    ) {
        panic!("an inheriting target is readable: {:?}", failure.failed);
    }
    Ok(())
}
