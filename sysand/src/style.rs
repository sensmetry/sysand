// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use clap::builder::styling::{AnsiColor, Effects, Style, Styles};

use sysand_core::style::Config;

pub const HEADER: Style = AnsiColor::Green.on_default().effects(Effects::BOLD);
pub const USAGE: Style = AnsiColor::Green.on_default().effects(Effects::BOLD);
pub const LITERAL: Style = AnsiColor::Cyan.on_default().effects(Effects::BOLD);
pub const PLACEHOLDER: Style = AnsiColor::Cyan.on_default();
pub const ERROR: Style = AnsiColor::Red.on_default().effects(Effects::BOLD);
pub const WARN: Style = AnsiColor::Yellow.on_default().effects(Effects::BOLD);
pub const NOTE: Style = AnsiColor::Cyan.on_default().effects(Effects::BOLD);
pub const GOOD: Style = AnsiColor::Green.on_default().effects(Effects::BOLD);
pub const VALID: Style = AnsiColor::Cyan.on_default().effects(Effects::BOLD);
pub const INVALID: Style = AnsiColor::Yellow.on_default().effects(Effects::BOLD);

pub const STYLING: Styles = Styles::styled()
    .header(HEADER)
    .usage(USAGE)
    .literal(LITERAL)
    .placeholder(PLACEHOLDER)
    .error(ERROR)
    .valid(VALID)
    .invalid(INVALID);

pub const CONFIG: Config = Config {
    header: HEADER,
    usage: USAGE,
    literal: LITERAL,
    placeholder: PLACEHOLDER,
    error: ERROR,
    warn: WARN,
    note: NOTE,
    good: GOOD,
    valid: VALID,
    invalid: INVALID,
};
