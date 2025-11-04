// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use env_logger::{Builder, Target, fmt::Formatter};
use log::{LevelFilter, Record};
use std::io::Write;

// use crate::cli;
use crate::style;

pub fn init(level: LevelFilter) {
    Builder::new()
        .filter_module("pubgrub", LevelFilter::Warn)
        .filter_level(level)
        .format(format)
        .target(Target::Stderr)
        .init();
}

fn format(buf: &mut Formatter, record: &Record<'_>) -> Result<(), std::io::Error> {
    match record.level() {
        log::Level::Error => {
            let style = style::ERROR;
            writeln!(buf, "{style}error{style:#}: {}", record.args())
        }
        log::Level::Warn => {
            let style = style::WARN;
            writeln!(buf, "{style}warning{style:#}: {}", record.args())
        }
        _ => {
            writeln!(buf, "{}", record.args())
        }
    }
}

const SP: char = ' ';

/// Print a warning that standard library package 'iri' is ignored
pub fn warn_std(iri: &str) {
    log::warn!(
        "SysML/KerML standard library package `{iri}` is ignored\n\
        {SP:>8} by default. If you want to process it, pass `--include-std` flag"
    );
}

/// Print a warning that standard library packages are omitted from output
pub fn warn_std_omit() {
    log::warn!(
        "SysML/KerML standard library packages are omitted by default.\n\
        {SP:>8} If you want to include them, pass `--include-std` flag"
    );
}

/// Print a warning that dependencies on standard library packages are ignored
pub fn warn_std_deps() {
    log::warn!(
        "Direct or transitive usages of SysML/KerML standard library packages are\n\
        {SP:>8} ignored by default. If you want to process them, pass `--include-std` flag"
    );
}
