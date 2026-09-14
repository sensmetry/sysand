// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The `model.toml` manifest: its value types and the diagnostics reading
//! one produces.

pub mod common;
pub mod diagnostics;
pub mod inherit;
pub mod reader;
pub mod serde_support;
pub mod spans;
pub mod usage;
pub mod workspace_table;
