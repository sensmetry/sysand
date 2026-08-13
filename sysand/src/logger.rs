// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use env_logger::{Builder, Target, fmt::Formatter};
use log::{LevelFilter, Record, SetLoggerError};
use std::io::{self, Write as _};
use sysand_core::utils::SP;

use crate::style::{self, GOOD};

pub fn init(level: LevelFilter) -> Result<(), SetLoggerError> {
    Builder::new()
        .filter_module("pubgrub", LevelFilter::Warn)
        .filter_level(level)
        .format(format)
        .target(Target::Stderr)
        .parse_default_env()
        .try_init()
}

fn format(buf: &mut Formatter, record: &Record<'_>) -> Result<(), io::Error> {
    match record.level() {
        log::Level::Error => {
            let style = style::ERROR;
            writeln!(buf, "{style}error{style:#}: {}", record.args())
        }
        log::Level::Warn => {
            let style = style::WARN;
            writeln!(buf, "{style}warning{style:#}: {}", record.args())
        }
        log::Level::Debug => {
            let style = style::NOTE;
            writeln!(buf, "{style}debug{style:#}: {}", record.args())
        }
        log::Level::Trace => {
            let style = style::PLACEHOLDER;
            writeln!(buf, "{style}trace{style:#}: {}", record.args())
        }
        log::Level::Info => {
            writeln!(buf, "{}", record.args())
        }
    }
}

/// Print a warning that dependencies on standard library packages are ignored
pub fn note_std_deps_no_install() {
    log::info!(
        "{GOOD}note{GOOD:#}: SysML v2/KerML standard library packages will not be installed during sync,\n\
        {SP:>5} since they should be provided by the tooling. If you want to install them,\n\
        {SP:>5} pass `--include-std` flag"
    );
}
