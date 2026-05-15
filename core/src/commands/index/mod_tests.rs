// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{fs, io::Write as _};

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use serde_json::{Value, json};
use zip::{DateTime, write::SimpleFileOptions};

use crate::{
    index::{
        add::IndexAddError, do_index_add, do_index_init, do_index_remove, do_index_yank,
        yank::IndexYankError,
    },
    project::utils::wrapfs,
};

#[test]
fn command_test() {
    let cwd = tempdir().unwrap();

    let kpar_path1 = cwd.path().join("test1.kpar");
    let iri = "pkg:sysand/dummy-publisher/dummy.name";
    write_kpar(
        &kpar_path1,
        "Dummy Publisher",
        "dummy.Name",
        "1.2.3",
        "0000-00-00T00:00:00.123456789Z",
        json!([]),
    );
    let kpar_path2 = cwd.path().join("test2.kpar");
    write_kpar(
        &kpar_path2,
        "Dummy publisher",
        "Dummy.name",
        "2.2.3",
        "0000-00-00T00:00:00.123456789Z",
        json!([]),
    );
    let kpar_path3 = cwd.path().join("test3.kpar");
    write_kpar(
        &kpar_path3,
        "dummy Publisher",
        "dummy.name",
        "3.2.3",
        "0000-00-00T00:00:00.123456789Z",
        json!([]),
    );

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

#[test]
fn file_state_test() {
    let cwd_dir = tempdir().unwrap();
    let cwd = cwd_dir.path();
    // let cwd = Utf8PathBuf::from("../temp/test");
    let kpar1v1_path = cwd.join("project1_0.1.0.kpar");
    write_kpar(
        &kpar1v1_path,
        "Test Publisher",
        "Test.project1",
        "0.1.0",
        "2026-05-15T12:35:57.053279000Z",
        json!([]),
    );
    let kpar1v2_path = cwd.join("project1_0.2.0.kpar");
    write_kpar(
        &kpar1v2_path,
        "Test Publisher",
        "Test.project1",
        "0.2.0",
        "2026-05-15T12:38:17.758551000Z",
        json!([]),
    );
    let kpar2v1_path = cwd.join("project2_0.1.0.kpar");
    write_kpar(
        &kpar2v1_path,
        "Test Publisher",
        "Test.project2",
        "0.1.0",
        "2026-05-15T12:42:04.424095000Z",
        json!([
          {
            "resource": "pkg:sysand/test-publisher/test.project1",
            "versionConstraint": "^0.1.0"
          }
        ]),
    );
    let index_root = cwd.join("index");
    do_index_init(&index_root).unwrap();
    do_index_add::<_, _, &str>(&index_root, &kpar1v1_path, None).unwrap();
    do_index_add::<_, _, &str>(&index_root, &kpar1v2_path, None).unwrap();
    do_index_add::<_, _, &str>(&index_root, &kpar2v1_path, None).unwrap();
    assert_eq!(
        read_json(index_root.join("index.json")),
        json!({
          "projects": [
            {
              "iri": "pkg:sysand/test-publisher/test.project1"
            },
            {
              "iri": "pkg:sysand/test-publisher/test.project2"
            }
          ]
        })
    );
    let project1_path = index_root.join("test-publisher/test.project1");
    assert_eq!(
        read_json(project1_path.join("versions.json")),
        json!({
          "versions": [
            {
              "version": "0.2.0",
              "usage": [],
              "project_digest": "sha256:6420d5d3170a11b6f6a811dfa71940317e69cef249ce664c1e4499124676fdd6",
              "kpar_size": 348,
              "kpar_digest": "sha256:873476ac47fe239c60d7ed6a51d752ae716d782872292ee7c7820cc3ee7fc021"
            },
            {
              "version": "0.1.0",
              "usage": [],
              "project_digest": "sha256:de024b833722716ad706981bdcb809f9af28e609ccee7c6522567218ca7fe2a6",
              "kpar_size": 348,
              "kpar_digest": "sha256:b67db84b3a2168e012262bd3dd7a202b284deb4f515a1418409d9b10d0effc8f"
            }
          ]
        })
    );
    let project1v1_path = project1_path.join("0.1.0");
    assert_eq!(
        read_json(project1v1_path.join(".project.json")),
        json!({
          "name": "Test.project1",
          "publisher": "Test Publisher",
          "version": "0.1.0",
          "usage": []
        })
    );
    assert_eq!(
        read_json(project1v1_path.join(".meta.json")),
        json!({
          "index": {},
          "created": "2026-05-15T12:35:57.053279000Z"
        })
    );
    assert_eq!(
        fs::read(project1v1_path.join("project.kpar")).unwrap(),
        fs::read(kpar1v1_path).unwrap()
    );
    let project1v2_path = project1_path.join("0.2.0");
    assert_eq!(
        read_json(project1v2_path.join(".project.json")),
        json!({
          "name": "Test.project1",
          "publisher": "Test Publisher",
          "version": "0.2.0",
          "usage": []
        })
    );
    assert_eq!(
        read_json(project1v2_path.join(".meta.json")),
        json!({
          "index": {},
          "created": "2026-05-15T12:38:17.758551000Z"
        })
    );
    assert_eq!(
        fs::read(project1v2_path.join("project.kpar")).unwrap(),
        fs::read(kpar1v2_path).unwrap()
    );

    let project2_path = index_root.join("test-publisher/test.project2");
    assert_eq!(
        read_json(project2_path.join("versions.json")),
        json!({
          "versions": [
            {
              "version": "0.1.0",
              "usage": [
                {
                  "resource": "pkg:sysand/test-publisher/test.project1",
                  "versionConstraint": "^0.1.0"
                }
              ],
              "project_digest": "sha256:6606158ab6f322fe25b9c2f8d963fa30ececf5156c5e5570185f6896aa4ea452",
              "kpar_size": 397,
              "kpar_digest": "sha256:3acdae9db465a4edcf3d99c4a57bf476c9acf3045636c6b8bb091db8cf61bdbe"
            }
          ]
        })
    );
    let project2v1_path = project2_path.join("0.1.0");
    assert_eq!(
        read_json(project2v1_path.join(".project.json")),
        json!({
          "name": "Test.project2",
          "publisher": "Test Publisher",
          "version": "0.1.0",
          "usage": [
            {
              "resource": "pkg:sysand/test-publisher/test.project1",
              "versionConstraint": "^0.1.0"
            }
          ]
        })
    );
    assert_eq!(
        read_json(project2v1_path.join(".meta.json")),
        json!({
          "index": {},
          "created": "2026-05-15T12:42:04.424095000Z"
        })
    );
    assert_eq!(
        fs::read(project2v1_path.join("project.kpar")).unwrap(),
        fs::read(kpar2v1_path).unwrap()
    );
}

fn write_kpar(
    kpar_path: &Utf8Path,
    publisher: &str,
    name: &str,
    version: &str,
    created: &str,
    usage: Value,
) {
    let info = json!({"name": name, "publisher": publisher, "version": version, "usage": usage});
    let meta = json!({"index":{}, "created":created});

    let file = wrapfs::File::create(kpar_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);

    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .system(zip::System::Unix)
        .last_modified_time(DateTime::DEFAULT);

    println!("{}", serde_json::to_string(&info).unwrap());
    zip.start_file(".project.json", options).unwrap();
    zip.write_all(serde_json::to_string(&info).unwrap().as_bytes())
        .unwrap();
    println!("{}", serde_json::to_string(&meta).unwrap());
    zip.start_file(".meta.json", options).unwrap();
    zip.write_all(serde_json::to_string(&meta).unwrap().as_bytes())
        .unwrap();

    zip.finish().unwrap();
}

fn read_json(path: Utf8PathBuf) -> Value {
    let str = wrapfs::read_to_string(path).unwrap();
    serde_json::from_str::<Value>(&str).unwrap()
}
