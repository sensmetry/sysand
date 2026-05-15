// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::io::Write as _;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use serde_json::json;
use zip::write::SimpleFileOptions;

use crate::index::{
    add::IndexAddError, do_index_add, do_index_init, do_index_remove, do_index_yank,
    to_json_string, yank::IndexYankError,
};

fn write_kpar(kpar_path: &Utf8Path, publisher: &str, name: &str, version: &str) {
    let info = json!({"name": name, "publisher": publisher, "version": version, "usage": []});
    let meta = json!({"index":{},"created":"0000-00-00T00:00:00.123456789Z"});

    let file = std::fs::File::create(kpar_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);

    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);

    zip.start_file(".project.json", options).unwrap();
    zip.write_all(to_json_string(&info).as_bytes()).unwrap();
    zip.start_file(".meta.json", options).unwrap();
    zip.write_all(to_json_string(&meta).as_bytes()).unwrap();
    zip.start_file("test.sysml", options).unwrap();
    zip.write_all(br#"package Test;"#).unwrap();

    zip.finish().unwrap();
}

#[test]
fn test() {
    let cwd = tempdir().unwrap();

    let kpar_path1 = cwd.path().join("test1.kpar");
    let iri = "pkg:sysand/dummy-publisher/dummy.name";
    write_kpar(&kpar_path1, "Dummy Publisher", "dummy.Name", "1.2.3");
    let kpar_path2 = cwd.path().join("test2.kpar");
    write_kpar(&kpar_path2, "Dummy publisher", "Dummy.name", "2.2.3");
    let kpar_path3 = cwd.path().join("test3.kpar");
    write_kpar(&kpar_path3, "dummy Publisher", "dummy.name", "3.2.3");

    do_index_init(&cwd).unwrap();

    do_index_add::<_, _, &str>(&cwd, &kpar_path1, None).unwrap();
    {
        let add_err = do_index_add::<_, _, &str>(&cwd, &kpar_path1, None).unwrap_err();
        assert!(
            matches!(add_err, IndexAddError::VersionAlreadyExists { .. }),
            "this must be VersionAlreadyExists error: {add_err}"
        );
    }
    do_index_add::<_, _, &str>(&cwd, kpar_path2, None).unwrap();

    do_index_yank(&cwd, iri, "1.2.3").unwrap();
    {
        let yank_err = do_index_yank(&cwd, iri, "1.2.4").unwrap_err();
        assert!(
            matches!(yank_err, IndexYankError::VersionNotFound { .. }),
            "this must be VersionNotFound error: {yank_err}"
        );
    }
    {
        let add_err = do_index_add::<_, _, &str>(&cwd, &kpar_path1, None).unwrap_err();
        assert!(
            matches!(add_err, IndexAddError::VersionYanked { .. }),
            "this must be VersionYanked error: {add_err}"
        );
    }

    do_index_remove(&cwd, iri, Some("1.2.3")).unwrap();
    {
        let add_result = do_index_add::<_, _, &str>(&cwd, &kpar_path1, None).unwrap_err();
        assert!(
            matches!(add_result, IndexAddError::VersionRemoved { .. }),
            "this must be VersionRemoved error: {add_result}"
        );
    }
    {
        let yank_err = do_index_yank(&cwd, iri, "1.2.3").unwrap_err();
        assert!(
            matches!(yank_err, IndexYankError::VersionRemoved { .. }),
            "this must be VersionRemoved error: {yank_err}"
        );
    }

    do_index_remove::<_, _, &str>(&cwd, iri, None).unwrap();
    {
        let add_err = do_index_add::<_, _, &str>(&cwd, &kpar_path3, None).unwrap_err();
        assert!(
            matches!(add_err, IndexAddError::ProjectRemoved { .. }),
            "this must be ProjectRemoved error: {add_err}"
        );
    }
    {
        let yank_err = do_index_yank(&cwd, iri, "2.2.3").unwrap_err();
        assert!(
            matches!(yank_err, IndexYankError::VersionRemoved { .. }),
            "this must be VersionRemoved error: {yank_err}"
        );
    }
}
