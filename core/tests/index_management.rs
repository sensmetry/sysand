// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#![cfg(feature = "filesystem")]

use std::{fs, io::Write as _};

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

use sysand_core::{
    index::{RemoveTarget, do_index_add, do_index_init, do_index_remove, do_index_yank},
    utils::format_err,
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
        "0000-01-01T00:00:00.123456789Z",
        json!([]),
    );
    let kpar_path2 = cwd.path().join("test2.kpar");
    write_kpar(
        &kpar_path2,
        "Dummy Publisher",
        "dummy.Name",
        "2.2.3",
        "0000-01-01T00:00:00.123456789Z",
        json!([]),
    );
    let kpar_path3 = cwd.path().join("test3.kpar");
    write_kpar(
        &kpar_path3,
        "dummy Publisher",
        "dummy.name",
        "3.2.3",
        "0000-01-01T00:00:00.123456789Z",
        json!([]),
    );

    do_index_init(&cwd).unwrap();

    do_index_add::<&str, _, _>(None, &kpar_path1, &cwd).unwrap();
    {
        let add_err = do_index_add::<&str, _, _>(None, &kpar_path1, &cwd).unwrap_err();
        assert_error_contains(add_err, "already exists");
    }
    do_index_add::<&str, _, _>(None, kpar_path2, &cwd).unwrap();

    do_index_yank(iri, "1.2.3", &cwd).unwrap();
    {
        let yank_err = do_index_yank(iri, "1.2.4", &cwd).unwrap_err();
        assert_error_contains(yank_err, "does not exist");
    }
    {
        let add_err = do_index_add::<&str, _, _>(None, &kpar_path1, &cwd).unwrap_err();
        assert_error_contains(add_err, "is yanked");
    }

    do_index_remove(iri, RemoveTarget::Version("1.2.3".to_owned()), &cwd).unwrap();
    {
        let add_result = do_index_add::<&str, _, _>(None, &kpar_path1, &cwd).unwrap_err();
        assert_error_contains(add_result, "is removed");
    }
    {
        let yank_err = do_index_yank(iri, "1.2.3", &cwd).unwrap_err();
        assert_error_contains(yank_err, "is removed");
    }

    do_index_remove(iri, RemoveTarget::Project, &cwd).unwrap();
    {
        let add_err = do_index_add::<&str, _, _>(None, &kpar_path3, &cwd).unwrap_err();
        assert_error_contains(add_err, "is removed");
    }
    {
        let yank_err = do_index_yank(iri, "2.2.3", &cwd).unwrap_err();
        assert_error_contains(yank_err, "is removed");
    }
}

#[test]
fn file_state_test() {
    let cwd_dir = tempdir().unwrap();
    let cwd = cwd_dir.path();
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
    do_index_add::<&str, _, _>(None, &kpar1v1_path, &index_root).unwrap();
    do_index_add::<&str, _, _>(None, &kpar1v2_path, &index_root).unwrap();
    do_index_add::<&str, _, _>(None, &kpar2v1_path, &index_root).unwrap();
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
              "kpar_size": 348,
              "kpar_digest": "sha256:873476ac47fe239c60d7ed6a51d752ae716d782872292ee7c7820cc3ee7fc021"
            },
            {
              "version": "0.1.0",
              "usage": [],
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
          "version": "0.1.0"
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
          "version": "0.2.0"
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

/// Every version of a project in an index spells its publisher and name the
/// same way
#[test]
fn spelling_is_kept_per_project() {
    let cwd_dir = tempdir().unwrap();
    let cwd = cwd_dir.path();
    let iri = "pkg:sysand/acme-labs/my-lib";
    let kpar = |file: &str, publisher: &str, name: &str, version: &str| {
        let path = cwd.join(file);
        write_kpar(
            &path,
            publisher,
            name,
            version,
            "2026-05-15T12:35:57.053279000Z",
            json!([]),
        );
        path
    };
    let v1 = kpar("v1.kpar", "Acme Labs", "My Lib", "1.0.0");
    let v2 = kpar("v2.kpar", "Acme Labs", "My Lib", "2.0.0");
    let v2_other = kpar("v2_other.kpar", "ACME Labs", "My Lib", "2.0.0");
    let v3_other = kpar("v3_other.kpar", "Acme Labs", "my lib", "3.0.0");
    let index_root = cwd.join("index");
    do_index_init(&index_root).unwrap();

    do_index_add::<&str, _, _>(None, &v1, &index_root).unwrap();
    let versions_before = read_json(index_root.join("acme-labs/my-lib/versions.json"));
    let err = do_index_add::<&str, _, _>(None, &v2_other, &index_root).unwrap_err();
    assert_error_contains(
        err,
        &format!(
            "{iri} is spelled `Acme Labs/My Lib` by the versions already in the index (1.0.0),\n\
             but version 2.0.0 in `{v2_other}` spells it `ACME Labs/My Lib`"
        ),
    );
    // Nothing was added
    assert_eq!(
        read_json(index_root.join("acme-labs/my-lib/versions.json")),
        versions_before
    );
    assert!(!index_root.join("acme-labs/my-lib/2.0.0").exists());

    // Yanked versions count, but removed ones are gone
    do_index_add::<&str, _, _>(None, &v2, &index_root).unwrap();
    do_index_yank(iri, "2.0.0", &index_root).unwrap();
    let err = do_index_add::<&str, _, _>(None, &v3_other, &index_root).unwrap_err();
    assert_error_contains(err, "by the versions already in the index (2.0.0, 1.0.0)");
    do_index_remove(iri, RemoveTarget::Version("1.0.0".to_owned()), &index_root).unwrap();
    do_index_remove(iri, RemoveTarget::Version("2.0.0".to_owned()), &index_root).unwrap();
    do_index_add::<&str, _, _>(None, &v3_other, &index_root).unwrap();
}

/// An index whose versions of a project already disagree on the spelling
/// refuses to add another version of it
#[test]
fn inconsistently_spelled_index_is_refused() {
    let cwd_dir = tempdir().unwrap();
    let cwd = cwd_dir.path();
    let kpar = |file: &str, version: &str| {
        let path = cwd.join(file);
        write_kpar(
            &path,
            "Acme Labs",
            "My Lib",
            version,
            "2026-05-15T12:35:57.053279000Z",
            json!([]),
        );
        path
    };
    let index_root = cwd.join("index");
    do_index_init(&index_root).unwrap();
    do_index_add::<&str, _, _>(None, kpar("v1.kpar", "1.0.0"), &index_root).unwrap();
    do_index_add::<&str, _, _>(None, kpar("v2.kpar", "2.0.0"), &index_root).unwrap();
    // As an index built before the spelling was kept would be
    let info_path = index_root.join("acme-labs/my-lib/2.0.0/.project.json");
    let mut info = read_json(info_path.clone());
    info["publisher"] = json!("ACME Labs");
    fs::write(&info_path, info.to_string()).unwrap();

    let err = do_index_add::<&str, _, _>(None, kpar("v3.kpar", "3.0.0"), &index_root).unwrap_err();
    assert_error_contains(
        err,
        "the versions of pkg:sysand/acme-labs/my-lib already in the index spell its publisher \
         and name differently: `ACME Labs/My Lib` (2.0.0), `Acme Labs/My Lib` (1.0.0);\n\
         no version can be added until they agree",
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

    let file = fs::File::create(kpar_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);

    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .system(zip::System::Unix)
        .last_modified_time(zip::DateTime::DEFAULT);

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
    let str = fs::read_to_string(path).unwrap();
    serde_json::from_str::<Value>(&str).unwrap()
}

fn assert_error_contains<E: std::error::Error>(err: E, expected: &str) {
    let err = format_err(err);
    assert!(
        err.contains(expected),
        "expected error to contain `{expected}`, got `{err}`"
    );
}
