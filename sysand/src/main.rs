// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::panic;

use anstream::{eprint, eprintln};
use clap::Parser;

use sysand::{cli::Args, run_cli};

fn main() {
    set_panic_hook();

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
            err.print().expect("failed to write Clap error");
            std::process::exit(err.exit_code())
        }
    }
}

fn set_panic_hook() {
    // TODO: use `panic::update_hook()` once it's stable
    //       also set bactrace style once it's stable, but take
    //       into account the current level
    let default_hook = panic::take_hook();
    // panic::set_backtrace_style(panic::BacktraceStyle::Short);
    panic::set_hook(Box::new(move |panic_info| {
        std::eprintln!(
            "Sysand crashed. This is a bug. We would appreciate a bug report at either\n\
            Sysand's issue tracker: https://github.com/sensmetry/sysand/issues\n\
            or Sensmetry forum: https://forum.sensmetry.com/c/sysand/24\n\
            or via email: sysand@sensmetry.com\n\
            \n\
            Below are details of the crash. It would be helpful to include them in the bug report."
        );
        default_hook(panic_info);
    }));
}
