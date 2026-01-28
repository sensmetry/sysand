// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{env, process::ExitCode};

use sysand::lib_main;

fn main() -> ExitCode {
    lib_main(env::args_os())
}
