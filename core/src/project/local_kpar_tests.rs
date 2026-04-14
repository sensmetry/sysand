use std::io::{Read as _, Write};

use camino_tempfile::tempdir;
use zip::write::SimpleFileOptions;

use super::ProjectRead;

#[test]
fn test_basic_kpar_archive() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempdir()?;
    let zip_path = cwd.path().join("test.kpar");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file(".project.json", options)?;
        zip.write_all(br#"{"name":"test_basic_kpar_archive","version":"1.2.3","usage":[]}"#)?;
        zip.start_file(".meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"123"}"#)?;
        zip.start_file("test.sysml", options)?;
        zip.write_all(br#"package Test;"#)?;

        zip.finish().unwrap();
    }

    let project = super::LocalKParProject::new_guess_root(zip_path)?;

    let (Some(info), Some(meta)) = project.get_project()? else {
        panic!();
    };

    assert_eq!(info.name, "test_basic_kpar_archive");
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
fn test_nested_kpar_archive() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempdir()?;
    let zip_path = cwd.path().join("test.kpar");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file("some_root_dir/.project.json", options)?;
        zip.write_all(br#"{"name":"test_nested_kpar_archive","version":"1.2.3","usage":[]}"#)?;
        zip.start_file("some_root_dir/.meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"123"}"#)?;
        zip.start_file("some_root_dir/test.sysml", options)?;
        zip.write_all(br#"package Test;"#)?;

        zip.finish().unwrap();
    }

    let project = super::LocalKParProject::new_guess_root(zip_path)?;

    let (Some(info), Some(meta)) = project.get_project()? else {
        panic!();
    };

    assert_eq!(info.name, "test_nested_kpar_archive");
    assert_eq!(info.version, "1.2.3");
    assert_eq!(meta.created, "123");

    let mut src = String::new();
    project
        .read_source("test.sysml")?
        .read_to_string(&mut src)?;

    assert_eq!(src, "package Test;");

    Ok(())
}
