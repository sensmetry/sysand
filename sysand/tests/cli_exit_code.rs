// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `lib_main` produces the process's exit code as a number, so an embedder
//! can pass it on. `std::process::ExitCode` can be returned from `main` but
//! not inspected, which is why `main.rs` does the wrapping rather than this.
//!
//! Parse-level cases only: they depend on neither the current directory nor
//! the environment, so they are safe in a parallel in-process harness. The
//! runtime-failure code is asserted from the Python suite, where each test
//! already gets an isolated directory.

use sysand::lib_main;

#[test]
fn version_is_success() {
    assert_eq!(lib_main(["sysand", "--version"]), 0);
}

#[test]
fn unknown_flag_is_claps_usage_code() {
    assert_eq!(lib_main(["sysand", "--not-a-flag"]), 2);
}

#[test]
fn no_arguments_is_claps_usage_code() {
    // `arg_required_else_help`, which clap reports as an error with the same
    // usage code rather than as success.
    assert_eq!(lib_main(["sysand"]), 2);
}
