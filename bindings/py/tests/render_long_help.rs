// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use pyo3::prelude::*;

use sysand_py::sysand_py;

#[test]
fn renders_help_for_a_foreign_program_name() -> Result<(), Box<dyn std::error::Error>> {
    pyo3::append_to_inittab!(sysand_py);
    Python::initialize();

    Python::attach(|py| -> PyResult<()> {
        let render = py.import("_sysand_core")?.getattr("_render_long_help")?;

        let root: String = render.call1(("sysand", Vec::<String>::new()))?.extract()?;
        assert!(root.contains("Usage: sysand"), "{root}");

        let nested: String = render.call1(("custom-sysand", vec!["env"]))?.extract()?;
        assert!(nested.contains("Usage: custom-sysand env"), "{nested}");

        Ok(())
    })?;

    Ok(())
}
