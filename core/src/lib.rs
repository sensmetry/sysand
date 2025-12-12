// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(refining_impl_trait)]

pub mod commands;
pub use commands::*;

pub mod model;

pub mod config;
pub mod env;
pub mod lock;
pub mod project;
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
mod tests {
    //use crate::{Message, get_message};

    #[test]
    fn placeholder_test() {
        assert_eq!(1, 1);
    }
}
