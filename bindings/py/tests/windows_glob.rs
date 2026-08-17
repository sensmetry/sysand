// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Unlike POSIX shells, `cmd.exe` and PowerShell do not expand glob patterns
//! (`*`, `?`, ...) in arguments before invoking a program; `_run_cli` expands
//! them itself (via the `glob` crate), but only on Windows.

use std::fs;

use pyo3::prelude::*;

use camino_tempfile::Utf8TempDir;
use sysand_py::sysand_py;

#[test]
fn run_cli_expands_glob_pattern() -> Result<(), Box<dyn std::error::Error>> {
    let proj_dir = Utf8TempDir::new()?;
    let proj_dir_path = proj_dir.path();

    fs::write(proj_dir_path.join("p1.sysml"), b"package P1;\n")?;
    fs::write(proj_dir_path.join("p2.sysml"), b"package P2;\n")?;
    fs::write(proj_dir_path.join("p3.sysml"), b"package P3;\n")?;
    // Should not be picked up by the `*.sysml` glob below.
    fs::write(proj_dir_path.join("readme.txt"), b"not a model file\n")?;

    // `_run_cli` glob-expands relative to CWD.
    std::env::set_current_dir(proj_dir_path)?;

    pyo3::append_to_inittab!(sysand_py);
    Python::initialize();

    let success: bool = Python::attach(|py| -> PyResult<bool> {
        let core = py.import("_sysand_core")?;

        core.getattr("do_init_py_local_file")?.call1((
            "windows_glob",
            "a",
            "1.0.0",
            proj_dir_path.as_str(),
        ))?;

        // Passed as a single, unexpanded argument, exactly as `cmd.exe` and
        // PowerShell would hand it to us.
        core.getattr("_run_cli")?
            .call1((vec!["sysand", "include", "*.sysml"],))?
            .extract()
    })?;

    let meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    if cfg!(target_os = "windows") {
        assert!(success);
        assert!(meta.contains(r#""P1": "p1.sysml""#), "meta.json: {meta}");
        assert!(meta.contains(r#""P2": "p2.sysml""#), "meta.json: {meta}");
        assert!(meta.contains(r#""P3": "p3.sysml""#), "meta.json: {meta}");
        assert!(!meta.contains("readme.txt"), "meta.json: {meta}");
    } else {
        // `_run_cli` only expands globs on Windows, so `include` sees a
        // literal `*.sysml` path, which doesn't exist.
        assert!(!success);
        assert!(meta.contains(r#""index": {}"#), "meta.json: {meta}");
    }

    Ok(())
}
