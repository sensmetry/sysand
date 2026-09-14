// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;
use zip::write::SimpleFileOptions;

use std::assert_matches;
use std::io::{Read as _, Write as _};

use crate::project::local_kpar::KparInnerPath;

use super::ProjectRead as _;

#[test]
fn basic_kpar_archive() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempdir()?;
    let zip_path = cwd.path().join("test.kpar");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file(".project.json", options)?;
        zip.write_all(br#"{"name":"basic_kpar_archive","version":"1.2.3"}"#)?;
        zip.start_file(".meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"123"}"#)?;
        zip.start_file("test.sysml", options)?;
        zip.write_all(b"package Test;")?;

        zip.finish().unwrap();
    }

    let project = super::LocalKParProject::new_access(zip_path, KparInnerPath::Guess, None);

    let (Some(info), Some(meta)) = project.get_project()? else {
        panic!();
    };

    assert_eq!(info.name, "basic_kpar_archive");
    assert_eq!(info.version, "1.2.3");
    assert_eq!(meta.created, "123");

    let mut src = String::new();
    project
        .read_source("test.sysml")?
        .read_to_string(&mut src)?;

    assert_eq!(src, "package Test;");

    Ok(())
}

#[test]
fn nested_kpar_archive() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempdir()?;
    let zip_path = cwd.path().join("test.kpar");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file("some_root_dir/.project.json", options)?;
        zip.write_all(br#"{"name":"nested_kpar_archive","version":"1.2.3"}"#)?;
        zip.start_file("some_root_dir/.meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"123"}"#)?;
        zip.start_file("some_root_dir/test.sysml", options)?;
        zip.write_all(b"package Test;")?;

        zip.finish().unwrap();
    }

    let project = super::LocalKParProject::new_access(zip_path, KparInnerPath::Guess, None);

    let (Some(info), Some(meta)) = project.get_project()? else {
        panic!();
    };

    assert_eq!(info.name, "nested_kpar_archive");
    assert_eq!(info.version, "1.2.3");
    assert_eq!(meta.created, "123");

    let mut src = String::new();
    project
        .read_source("test.sysml")?
        .read_to_string(&mut src)?;

    assert_eq!(src, "package Test;");

    Ok(())
}

#[test]
fn project_root_uses_zip_path_separators() {
    let root = super::project_root_from_zip_entry_path(typed_path::Utf8UnixPath::new(
        "some_root_dir/.project.json",
    ))
    .expect("valid archive path")
    .expect("project info file");

    assert_eq!(root, typed_path::Utf8UnixPath::new("some_root_dir"));
}

/// A KPAR whose metadata is missing a key names the key, as a project
/// directory does.
#[test]
fn kpar_with_a_malformed_meta_names_the_key() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempdir()?;
    let zip_path = cwd.path().join("test.kpar");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file(".project.json", options)?;
        zip.write_all(br#"{"name":"p","version":"1.2.3"}"#)?;
        zip.start_file(".meta.json", options)?;
        zip.write_all(br#"{"created":"123"}"#)?;
        zip.finish().unwrap();
    }

    let project = super::LocalKParProject::new_access(zip_path, KparInnerPath::Guess, None);
    let error = project
        .get_project()
        .expect_err("the metadata is not valid")
        .to_string();

    assert!(
        error.contains("missing key `index` in `.meta.json`"),
        "unexpected error: {error}"
    );
    Ok(())
}

/// Write a `.kpar` at `path` holding `entries`.
fn kpar(
    path: &camino::Utf8Path,
    entries: &[(&str, &str)],
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);
    for (name, content) in entries {
        zip.start_file(*name, options)?;
        zip.write_all(content.as_bytes())?;
    }
    zip.finish()?;
    Ok(())
}

const INFO: &str = r#"{"name":"widgets","version":"1.2.3"}"#;
const META: &str = r#"{"index":{},"created":"1970-01-01T00:00:00Z"}"#;

/// model-toml.md "Mixed manifests": "if toml config present in a KPAR,
/// error out". Checked on every way in, since all three read the archive
/// separately.
#[test]
fn a_kpar_with_a_model_toml_beside_the_pair_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempdir()?;
    let zip_path = cwd.path().join("test.kpar");
    kpar(
        &zip_path,
        &[
            (".project.json", INFO),
            (".meta.json", META),
            ("model.toml", "manifest-version = 1\n"),
        ],
    )?;

    let project = super::LocalKParProject::new_access(zip_path.clone(), KparInnerPath::Root, None);
    assert_eq!(
        project
            .get_project()
            .expect_err("a toml manifest")
            .to_string(),
        format!(
            "`{zip_path}` contains `model.toml`;\n         \
             a KPAR carries `.project.json` and `.meta.json`, never a `model.toml`"
        )
    );
    assert_matches!(
        project.get_info().expect_err("a toml manifest"),
        super::LocalKParError::TomlInKpar { .. }
    );
    assert_matches!(
        project.get_meta().expect_err("a toml manifest"),
        super::LocalKParError::TomlInKpar { .. }
    );

    Ok(())
}

/// An archive with no `.project.json` but a `model.toml` is refused as
/// a toml archive rather than with "`.project.json` not found", which
/// says nothing about an archive that plainly carries a manifest.
#[test]
fn a_kpar_with_only_a_model_toml_is_refused_by_the_guess() -> Result<(), Box<dyn std::error::Error>>
{
    let cwd = tempdir()?;
    let zip_path = cwd.path().join("test.kpar");
    kpar(&zip_path, &[("model.toml", "manifest-version = 1\n")])?;

    let project = super::LocalKParProject::new_access(zip_path, KparInnerPath::Guess, None);
    assert_matches!(
        project.get_project().expect_err("a toml manifest"),
        super::LocalKParError::TomlInKpar { .. }
    );

    Ok(())
}

/// A `model.toml` that is one of the project's own files is not its
/// manifest, so neither check fires.
#[test]
fn a_kpar_with_a_model_toml_among_its_sources_is_read() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempdir()?;
    let zip_path = cwd.path().join("test.kpar");
    kpar(
        &zip_path,
        &[
            ("examples/model.toml", "manifest-version = 1\n"),
            (".project.json", INFO),
            (".meta.json", META),
        ],
    )?;

    let project = super::LocalKParProject::new_access(zip_path, KparInnerPath::Guess, None);
    let (info, _) = project.get_project()?;
    assert_eq!(info.expect("`.project.json`").name, "widgets");

    Ok(())
}
