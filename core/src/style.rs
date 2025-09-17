// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::OnceLock;

use anstyle::Style;

pub struct Config {
    pub header: Style,
    pub usage: Style,
    pub literal: Style,
    pub placeholder: Style,
    pub error: Style,
    pub warn: Style,
    pub note: Style,
    pub good: Style,
    pub valid: Style,
    pub invalid: Style,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            header: Style::new(),
            usage: Style::new(),
            literal: Style::new(),
            placeholder: Style::new(),
            error: Style::new(),
            warn: Style::new(),
            note: Style::new(),
            good: Style::new(),
            valid: Style::new(),
            invalid: Style::new(),
        }
    }
}

static STYLE_CONFIG: OnceLock<Config> = OnceLock::new();

pub fn set_style_config(config: Config) {
    let _ = STYLE_CONFIG.set(config);
}

pub fn get_style_config() -> &'static Config {
    STYLE_CONFIG.get_or_init(Config::default)
}
