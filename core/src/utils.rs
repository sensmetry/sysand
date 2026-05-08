// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::error::Error;

pub fn format_sources(mut error: &dyn Error) -> String {
    let mut message = error.to_string();
    while let Some(source) = error.source() {
        writeln!(&mut message, "  caused by: {source}").unwrap();
        error = source;
    }
    message
}
