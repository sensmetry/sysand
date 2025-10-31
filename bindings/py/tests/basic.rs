// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use predicates::prelude::*;
use pyo3::prelude::*;

use sysand_py::sysand_py;
use tempfile::TempDir;

#[test]
fn test_basic_new() -> Result<(), Box<dyn std::error::Error>> {
    let proj_dir: TempDir = TempDir::new()?;
    let proj_dir_path = proj_dir.path();

    pyo3::append_to_inittab!(sysand_py);
    Python::initialize();
    Python::attach(|py| {
        let do_new_py_local_file_fn = py
            .import("_sysand_core")
            .expect("Failed to import _sysand_core")
            .getattr("do_new_py_local_file")
            .expect("Failed to get do_new_py_local_file function");

        do_new_py_local_file_fn
            .call1((
                "test_basic_new",
                "1.2.3",
                proj_dir_path.to_str().unwrap().to_string(),
            ))
            .unwrap();
    });

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.(\d{6}|\d{9})Z"\n}"#,
    )?;

    assert_eq!(
        info,
        "{\n  \"name\": \"test_basic_new\",\n  \"version\": \"1.2.3\",\n  \"usage\": []\n}"
    );

    assert!(meta_match.eval(&meta));

    Ok(())
}
