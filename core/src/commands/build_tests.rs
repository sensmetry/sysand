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

/// model-toml.md "TOML to KPAR": a local usage "turn[s] into a PURL
/// constrained to `^<version>`", which needs the target's manifest.
#[test]
fn a_local_usage_is_refused_for_now() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../dep\"\n",
    )?;
    let error = built(&dir, false).expect_err("a local usage cannot be rewritten yet");
    assert!(matches!(error, KParBuildError::TomlLocalUsage { .. }));
    let text = error.to_string();
    assert!(text.contains("is not supported yet"), "{text}");
    // Both ways forward, because there are two.
    assert!(text.contains("--allow-path-usage"), "{text}");
    Ok(())
}

/// "with it, local usages are emitted as the typed usages `Directory`
/// (source path) and `KparPath` (kpar path), which preserve the path as
/// written" -- which is what the view already produces, so no target is
/// read.
#[test]
fn a_local_usage_builds_under_allow_path_usage() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../dep\"\n",
    )?;
    let (info, _) = built(&dir, true)?;
    let rendered = format!("{:?}", info.usage[0]);
    assert!(rendered.contains("Directory"), "{rendered}");
    assert!(rendered.contains("../dep"), "{rendered}");
    Ok(())
}

/// "an index, source path or kpar path usage whose target carries
/// `stdlib`: the OMG IRI instead of the PURL". For an index usage the
/// IRI comes from embedded metadata, which is WP-6b.
#[test]
fn a_stdlib_index_usage_is_refused_for_now() -> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"OMG\"\nname = \"kerml\"\nversion = \"^1.0.0\"\n",
    )?;
    let error = built(&dir, false).expect_err("a stdlib usage cannot be rewritten yet");
    assert!(matches!(error, KParBuildError::TomlStdlibUsage { .. }));
    Ok(())
}

/// The flag is about path usages: "under `--allow-path-usage` a local
/// usage stays a typed path usage, stdlib target or not" says nothing
/// about an index usage.
#[test]
fn a_stdlib_index_usage_is_refused_under_allow_path_usage_too()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = toml_project(
        "\n[[inline-usages]]\npublisher = \"OMG\"\nname = \"kerml\"\nversion = \"^1.0.0\"\n",
    )?;
    let error = built(&dir, true).expect_err("the flag does not excuse a stdlib usage");
    assert!(matches!(error, KParBuildError::TomlStdlibUsage { .. }));
    Ok(())
}

/// A json project's `.project.json` and its `created` survive a build.
///
/// Not "byte for byte": the build always rewrites `checksum` -- values
/// and algorithm -- and, without `--keep-index`, `index`.
#[test]
fn a_json_build_keeps_its_created() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    std::fs::write(
        dir.path().join(".project.json"),
        br#"{"name": "widgets", "publisher": "acme", "version": "1.0.0", "usage": []}"#,
    )?;
    std::fs::write(
        dir.path().join(".meta.json"),
        br#"{"created": "2001-02-03T04:05:06Z", "index": {}, "checksum": {"a.sysml": {"value": "", "algorithm": "NONE"}}}"#,
    )?;
    std::fs::write(dir.path().join("a.sysml"), b"package A;\n")?;

    let (info, meta) = built(&dir, false)?;
    assert_eq!(meta.created, "2001-02-03T04:05:06Z");
    assert_eq!(info.publisher.as_deref(), Some("acme"));
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
