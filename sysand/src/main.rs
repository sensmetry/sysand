// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::process::ExitCode;

use sysand::lib_main;

fn main() -> ExitCode {
    // `args_os()` does not panic on invalid Unicode, and clap gives a nice error
    lib_main(wild::args_os())
}
