// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `render_long_help` gives an embedder the text clap would have written to
//! the process's streams.

use std::process::Command;

use sysand::render_long_help;

/// The bare binary, deliberately not `common::sysand_cmd_in_with`: that
/// helper appends `--no-config` after the caller's arguments and injects
/// environment, neither of which belongs in a byte comparison of help text.
fn binary_help() -> String {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("sysand"))
        .arg("--help")
        .output()
        .expect("failed to run the sysand binary");
    String::from_utf8(output.stdout).expect("help is not UTF-8")
}

/// What the binary calls itself: clap takes its `bin_name` from
/// `Path::file_name(argv[0])`, which keeps the `.exe` suffix on Windows. Only
/// the byte comparison needs this; every other test names the program itself.
fn binary_prog() -> String {
    assert_cmd::cargo::cargo_bin!("sysand")
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the binary path ends in a UTF-8 file name")
        .to_owned()
}

#[test]
fn root_help_is_the_long_about() {
    let help = render_long_help("sysand", Vec::<String>::new());

    assert!(
        help.contains("A package manager for SysML v2 and KerML"),
        "{help}"
    );
    // Only the *long* about carries the links.
    assert!(help.contains("https://docs.sysand.com/client/"), "{help}");
    assert!(help.contains("Usage: sysand"), "{help}");
}

#[test]
fn matches_what_the_binary_prints() {
    assert_eq!(
        render_long_help(&binary_prog(), Vec::<String>::new()),
        binary_help()
    );
}

#[test]
fn subcommand_path_is_resolved() {
    let help = render_long_help("sysand", ["env", "sources"]);

    assert!(help.contains("Usage: sysand env sources"), "{help}");
    assert!(
        help.contains("List source files for an installed project"),
        "{help}"
    );
    assert!(
        !help.contains("A package manager for SysML v2 and KerML"),
        "the root about must not appear in a subcommand's help: {help}"
    );
}

#[test]
fn program_name_replaces_argv0() {
    let root = render_long_help("custom sysand", Vec::<String>::new());
    assert!(root.contains("Usage: custom sysand"), "{root}");

    let nested = render_long_help("custom sysand", ["env"]);
    assert!(nested.contains("Usage: custom sysand env"), "{nested}");
}

#[test]
fn program_name_does_not_change_the_footer() {
    // `{name} v{version}` in the help template resolves through clap's
    // display name, never the bin name. Deliberate: the usage line says how
    // to invoke the command, the footer says which tool and version answers.
    let help = render_long_help("custom sysand", Vec::<String>::new());

    assert!(help.contains("Usage: custom sysand"), "{help}");
    assert!(help.contains("sysand v"), "{help}");
}

#[test]
fn trailing_var_arg_subcommand_still_renders_its_own_help() {
    // `new` takes `trailing_var_arg` + `allow_hyphen_values`, so an appended
    // `--help` is swallowed as one of its values and the command line parses
    // cleanly. Falling back to the root help there would be silently wrong.
    for args in [vec!["new"], vec!["new", "somewhere"]] {
        let help = render_long_help("sysand", args.clone());

        assert!(help.contains("Usage: sysand new"), "{args:?}: {help}");
        assert!(
            !help.contains("A package manager for SysML v2 and KerML"),
            "{args:?} fell back to the root help: {help}"
        );
    }
}

#[test]
fn unknown_subcommand_returns_claps_error() {
    let help = render_long_help("sysand", ["definitely-not-a-subcommand"]);

    assert!(help.contains("error:"), "{help}");
    assert!(help.contains("definitely-not-a-subcommand"), "{help}");
}

#[test]
fn no_arguments_is_not_a_panic() {
    assert!(!render_long_help("sysand", Vec::<String>::new()).is_empty());
}
