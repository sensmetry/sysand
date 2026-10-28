// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use pyo3::prelude::*;
use sysand_py::sysand_py;

/// Registers the module as `_sysand_core`; `append_to_inittab!` is unavailable under `abi3t`
pub fn import_core(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    let module = pyo3::wrap_pymodule!(sysand_py)(py).into_bound(py);
    py.import("sys")?
        .getattr("modules")?
        .set_item("_sysand_core", &module)?;
    Ok(module)
}
