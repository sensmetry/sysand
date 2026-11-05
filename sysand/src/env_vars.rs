// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

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

// The `SYSAND_HTTP_*` variables configure Sysand's own HTTP client, used for
// indexes and kpar downloads. Git sources are fetched by gix with its own
// HTTP client, which they do not affect.

/// How many seconds to wait for each read of an HTTP response before giving
/// up. A positive whole number; also caps the connect timeout.
pub const SYSAND_HTTP_TIMEOUT: &str = "SYSAND_HTTP_TIMEOUT";

/// How many seconds to wait for an HTTP connection to be established.
/// A positive whole number.
pub const SYSAND_HTTP_CONNECT_TIMEOUT: &str = "SYSAND_HTTP_CONNECT_TIMEOUT";

/// How many times to retry an idempotent HTTP request after a transient
/// failure. A whole number; `0` disables retries.
pub const SYSAND_HTTP_RETRIES: &str = "SYSAND_HTTP_RETRIES";

/// Comma separated list of hosts whose HTTPS certificates are not verified.
/// Corresponds to the `--https-insecure-hosts` command line argument. Each
/// entry is a host name or IP address only (IPv6 in brackets), without
/// scheme, port or path. Insecure.
pub const SYSAND_HTTPS_INSECURE_HOSTS: &str = "SYSAND_HTTPS_INSECURE_HOSTS";
