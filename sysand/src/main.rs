// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#![cfg_attr(test, allow(clippy::pedantic, clippy::restriction))]

use std::{env, process::ExitCode};

use sysand::lib_main;

fn main() -> ExitCode {
    lib_main(env::args_os())
}
