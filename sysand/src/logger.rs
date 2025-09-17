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
