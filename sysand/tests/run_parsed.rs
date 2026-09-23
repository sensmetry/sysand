// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `run_parsed` runs a command an embedder has already parsed.
//!
//! An embedder with a command line of its own parses into Sysand's types
//! itself, so these tests build the global options by hand and take the
//! command from clap, as such an embedder would. They run in this process,
//! which is the point: no binary is involved.

use camino_tempfile::tempdir;
use clap::Parser as _;
use sysand::{
    ProcessOwnership,
    cli::{Args, Command, GlobalOptions},
    run_parsed,
};

/// Global options with nothing turned on, and no configuration file read.
fn global_opts() -> GlobalOptions {
    GlobalOptions {
        verbose: false,
        quiet: true,
        no_config: true,
        config_file: None,
        help: None,
    }
}

fn command(args: &[&str]) -> Command {
    Args::try_parse_from(std::iter::once("sysand").chain(args.iter().copied()))
        .expect("a valid command line")
        .command
}

#[test]
fn runs_the_command_it_is_given() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("p");

    let init = command(&["init", path.as_str(), "--name", "p", "--publisher", "acme"]);
    let code = run_parsed(global_opts(), init, ProcessOwnership::Embedded);

    assert_eq!(code, 0);
    assert!(path.join(".project.json").is_file());
    assert!(path.join(".meta.json").is_file());
}

#[test]
fn a_failing_command_exits_with_one() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("nothing");

    let info = command(&["info", "--path", missing.as_str()]);
    let code = run_parsed(global_opts(), info, ProcessOwnership::Embedded);

    // The code `lib_main` reports for a command that fails after parsing.
    assert_eq!(code, 1);
}
