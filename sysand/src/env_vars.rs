// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Corresponds to the `--index` command line argument. Can consist of a single or
/// multiple (comma separated) list of URLs as additional indexes to use when
/// resolving dependencies.
pub const SYSAND_INDEX: &str = "SYSAND_INDEX";

/// Corresponds to the `--default-index` command line argument. Can consist of a single or
/// multiple (comma separated) list of URLs as default indexes that will override other
/// default indexes.
pub const SYSAND_DEFAULT_INDEX: &str = "SYSAND_DEFAULT_INDEX";

/// Corresponds to the `--config-file` command line argument. Should be a file path to
/// Sysand config file.
pub const SYSAND_CONFIG_FILE: &str = "SYSAND_CONFIG_FILE";

/// Corresponds to the `--no-config` command line argument. If set prevents reading of any
/// Sysand config files.
pub const SYSAND_NO_CONFIG: &str = "SYSAND_NO_CONFIG";
