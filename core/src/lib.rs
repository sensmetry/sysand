// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

// Let `#[derive(ProjectRead)]` expand its `::sysand_core::...` paths inside this crate.
extern crate self as sysand_core;

pub mod commands;
pub use commands::*;

pub mod model;

#[cfg(feature = "networking")]
pub mod auth;
pub mod cancel;
pub mod config;
pub mod context;
pub mod credential_store;
pub mod env;
mod format_migrate;
pub mod graph;
#[cfg(feature = "filesystem")]
pub mod index;
pub mod index_location;
mod iri_normalize;
pub mod lock;
pub mod manifest;
pub mod project;
pub mod purl;
pub mod redact;
pub mod resolve;
pub mod solve;
pub mod stdlib;
pub mod stdlib_compare;
pub mod style;
pub mod symbols;
pub mod utils;

#[cfg(feature = "filesystem")]
pub mod workspace;
#[cfg(feature = "filesystem")]
pub mod workspace_lock;

#[cfg(feature = "filesystem")]
pub mod discover;

#[cfg(test)]
#[path = "./lib_tests.rs"]
mod tests;
