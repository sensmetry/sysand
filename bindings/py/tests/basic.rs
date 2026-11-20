// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use pyo3::prelude::*;

use camino_tempfile::Utf8TempDir;
use sysand_core::test_fs::exists;
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
            .call1((
                "basic-init",
                "acme",
                "1.2.3",
                proj_dir_path.as_str(),
                "https://www.omg.org/spec/SysML/20250201",
                "model-toml",
            ))
            .unwrap();
    });

    // `init` writes a `model.toml`, through the binding as through the
    // CLI: the two json documents are what a build produces and what a
    // json project has.
    assert_eq!(
        std::fs::read_to_string(proj_dir_path.join("model.toml"))?,
        r#"manifest-version = "1.0"

[project]
publisher = "acme"
name = "basic-init"
version = "1.2.3"
sources = []
metamodel = "https://www.omg.org/spec/SysML/20250201"
"#
    );
    assert!(!exists(proj_dir_path.join(".project.json")));

    Ok(())
}
