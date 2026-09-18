// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#[cfg(all(not(debug_assertions), feature = "tls-aws-lc-rs", feature = "tls-ring"))]
compile_error!("`tls-aws-lc-rs` and `tls-ring` must not both be enabled in release builds");

#[cfg(not(any(debug_assertions, feature = "tls-aws-lc-rs", feature = "tls-ring")))]
compile_error!("one of `tls-aws-lc-rs` and `tls-ring` must be enabled in release builds");

use std::process::ExitCode;

use sysand::lib_main;

fn main() -> ExitCode {
    // `args_os()` does not panic on invalid Unicode, and clap gives a nice error
    lib_main(wild::args_os())
}
