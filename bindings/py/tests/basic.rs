// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use predicates::prelude::*;
use pyo3::prelude::*;

use camino_tempfile::Utf8TempDir;
mod common;

use common::import_core;

#[test]
fn basic_init() -> Result<(), Box<dyn std::error::Error>> {
    let proj_dir = Utf8TempDir::new()?;
    let proj_dir_path = proj_dir.path();

    Python::initialize();
    Python::attach(|py| {
        let do_init_py_local_file_fn = import_core(py)
            .expect("Failed to import _sysand_core")
            .getattr("do_init_py_local_file")
            .expect("Failed to get do_init_py_local_file function");

        do_init_py_local_file_fn
            .call1(("basic_init", "a", "1.2.3", proj_dir_path.as_str()))
            .unwrap();
    });

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z"\n}\n"#,
    )?;

    assert_eq!(
        info,
        r#"{
  "name": "basic_init",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    assert!(meta_match.eval(&meta));

    Ok(())
}
