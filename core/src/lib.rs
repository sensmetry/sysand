// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#![allow(refining_impl_trait)]

extern crate self as sysand_core;

pub mod commands;
pub use commands::*;

pub mod model;

#[cfg(feature = "networking")]
pub mod auth;
pub mod config;
pub mod context;
pub mod env;
pub mod lock;
pub mod project;
pub mod purl;
pub mod resolve;
pub mod solve;
pub mod stdlib;
pub mod style;
pub mod symbols;

#[cfg(feature = "filesystem")]
pub mod workspace;

#[cfg(feature = "filesystem")]
pub mod discover;

#[cfg(not(feature = "std"))]
compile_error!("`std` feature is currently required to build `sysand`");

// #[cfg(feature = "python")]
// use pyo3::prelude::*;

// #[cfg(feature = "js")]
// use wasm_bindgen::prelude::*;

// // #[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
// // #[cfg_attr(feature = "python", pyclass(eq, get_all))]
// // #[cfg_attr(feature = "js", wasm_bindgen())]
// // pub struct Message {
// //     #[cfg_attr(feature = "js", wasm_bindgen(skip))]
// //     pub content: String,
// // }

// // #[cfg(feature = "js")]
// // #[wasm_bindgen]
// // impl Message {
// //     #[wasm_bindgen(getter)]
// //     pub fn content(&self) -> String {
// //         self.content.clone()
// //     }
// // }

// // pub fn get_message<S: Into<String>>(s: S) -> Message {
// //     Message {
// //         content: format!("Hello, {}!", s.into()).to_string(),
// //     }
// // }

// Private tests

#[cfg(test)]
#[path = "./lib_tests.rs"]
mod tests;
