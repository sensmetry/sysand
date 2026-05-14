// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::io::Write as _;

use camino::Utf8PathBuf;
use camino_tempfile::tempdir;
use zip::write::SimpleFileOptions;

use crate::index::{add::IndexAddError, do_index_add, do_index_init};

#[test]
fn test() {
    let info = r#"{"name": "dummy-project", "publisher": "dummy-publisher", "version": "1.2.3", "usage": []}"#;
    let meta = r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#;

    let cwd = tempdir().unwrap();
    // set_current_dir(&cwd).unwrap();

    dbg!(&cwd.path());
    let kpar_path = Utf8PathBuf::from("test.kpar");

    {
        let file = std::fs::File::create(&kpar_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file(".project.json", options).unwrap();
        zip.write_all(info.as_bytes()).unwrap();
        zip.start_file(".meta.json", options).unwrap();
        zip.write_all(meta.as_bytes()).unwrap();
        zip.start_file("test.sysml", options).unwrap();
        zip.write_all(br#"package Test;"#).unwrap();

        zip.finish().unwrap();
    }

    // println!(
    //     "{}",
    //     wrapfs::read_to_string(cwd.path().join("index.json")).unwrap()
    // );
    do_index_init(&cwd).unwrap();
    // println!("{}", wrapfs::read_to_string("index.json").unwrap());
    do_index_add::<_, _, &str>(&cwd, &kpar_path, None).unwrap();
    // println!("{}", wrapfs::read_to_string("index.json").unwrap());
    let add_result = do_index_add::<_, _, &str>(&cwd, &kpar_path, None);
    assert!(
        matches!(add_result, Err(IndexAddError::VersionAlreadyExists { .. })),
        "{add_result:?} should be duplicate error"
    );
}
