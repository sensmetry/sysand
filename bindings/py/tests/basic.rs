// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

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
            .call1((
                "basic-init",
                "acme",
                "1.2.3",
                proj_dir_path.as_str(),
                "https://www.omg.org/spec/SysML/20250201",
            ))
            .unwrap();
    });

    // `init` writes a `model.toml`, through the binding as through the
    // CLI: the two json documents are what a build produces and what a
    // legacy project has.
    assert_eq!(
        std::fs::read_to_string(proj_dir_path.join("model.toml"))?,
        "manifest-version = \"1.0.0\"\n\n\
         [project]\n\
         publisher = \"acme\"\n\
         name = \"basic-init\"\n\
         version = \"1.2.3\"\n\
         sources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
    );
    assert!(!proj_dir_path.join(".project.json").exists());

    Ok(())
}
