// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use clap::Parser;
use sysand::{cli::Args, run_cli};

fn main() {
    match Args::try_parse() {
        Ok(args) => {
            if let Err(err) = run_cli(args) {
                let style = sysand::style::ERROR;
                eprint!("{style}error{style:#}: ");
                for cause in err.chain() {
                    eprintln!("{}", cause);
                }
                std::process::exit(1)
            }
        }
        Err(err) => {
            err.print().expect("Failed to write Clap error");
            std::process::exit(err.exit_code())
        }
    }
}
