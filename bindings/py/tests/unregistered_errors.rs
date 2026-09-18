// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The extension's exception classes are defined in Python and handed to it
//! by `sysand/_errors.py` at import time, so that the tree can be vendored
//! under another parent package. An embedder that imports `_sysand_core`
//! *without* that package — which is exactly what this test binary does —
//! must get a plain, readable `RuntimeError` saying the classes were never
//! registered, not a panic and not another installation's classes.

use camino_tempfile::Utf8TempDir;
use pyo3::{exceptions::PyRuntimeError, prelude::*};

use sysand_py::sysand_py;

#[test]
fn unregistered_error_classes_raise_a_readable_runtime_error()
-> Result<(), Box<dyn std::error::Error>> {
    let project_dir = Utf8TempDir::new()?;

    pyo3::append_to_inittab!(sysand_py);
    Python::initialize();

    Python::attach(|py| -> PyResult<()> {
        let core = py.import("_sysand_core")?;

        // No project exists in the directory, so this can only fail, and it
        // fails with `ProjectError` once the classes are registered.
        let err = core
            .getattr("do_set_usage_constraint_py")?
            .call1((project_dir.path().as_str(), "pkg:sysand/example", "1.0.0"))
            .expect_err("setting a constraint without a project must fail");

        assert!(
            err.is_instance_of::<PyRuntimeError>(py),
            "expected a RuntimeError, got {err}"
        );
        let message = err.to_string();
        // The note is hard-wrapped, so match against a single-spaced copy:
        // re-flowing that text must not break this test.
        let unwrapped = message.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            unwrapped.contains("exception classes have not been registered"),
            "the error should say the classes are unregistered, got: {message}"
        );

        Ok(())
    })?;

    Ok(())
}
