// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use pyo3::prelude::*;

mod common;

use common::import_core;

#[test]
fn renders_help_for_a_foreign_program_name() -> Result<(), Box<dyn std::error::Error>> {
    Python::initialize();

    Python::attach(|py| -> PyResult<()> {
        let render = import_core(py)?.getattr("_render_long_help")?;

        let root: String = render.call1(("sysand", Vec::<String>::new()))?.extract()?;
        assert!(root.contains("Usage: sysand"), "{root}");

        let nested: String = render.call1(("custom-sysand", vec!["env"]))?.extract()?;
        assert!(nested.contains("Usage: custom-sysand env"), "{nested}");

        Ok(())
    })?;

    Ok(())
}
