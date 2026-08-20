// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;
use zip::write::SimpleFileOptions;

use std::{
    assert_matches,
    io::{Read as _, Write as _},
};

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
fn expected_pub_name_check() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempdir()?;
    let zip_path = cwd.path().join("test.kpar");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file(".project.json", options)?;
        zip.write_all(
            br#"{"publisher":"acme","name":"expected_pub_name_check","version":"1.2.3"}"#,
        )?;
        zip.start_file(".meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"123"}"#)?;

        zip.finish().unwrap();
    }

    let project = super::LocalKParProject::new_for_solve(
        &zip_path,
        None,
        Some("acme".to_owned()),
        "expected_pub_name_check".to_owned(),
    );

    let (Some(info), Some(_meta)) = project.get_project()? else {
        panic!();
    };
    assert_eq!(info.name, "expected_pub_name_check");

    let mismatched = super::LocalKParProject::new_for_solve(
        &zip_path,
        None,
        Some("acme".to_owned()),
        "wrong-name".to_owned(),
    );
    assert_matches!(
        mismatched.get_info(),
        Err(super::LocalKParError::NameMismatch { .. })
    );

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
