// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{error::Error, fmt::Write as _};

pub mod scheme {
    #[cfg(feature = "filesystem")]
    use fluent_uri::component::Scheme;
    #[cfg(feature = "filesystem")]
    pub const SCHEME_FILE: &Scheme = Scheme::new_or_panic("file");
    #[cfg(all(feature = "filesystem", feature = "networking"))]
    pub const SCHEME_SSH: &Scheme = Scheme::new_or_panic("ssh");
    #[cfg(all(feature = "filesystem", feature = "networking"))]
    pub const SCHEME_GIT_SSH: &Scheme = Scheme::new_or_panic("git+ssh");
    #[cfg(all(feature = "filesystem", feature = "networking"))]
    pub const SCHEME_GIT_FILE: &Scheme = Scheme::new_or_panic("git+file");
    #[cfg(all(feature = "filesystem", feature = "networking"))]
    pub const SCHEME_GIT_HTTP: &Scheme = Scheme::new_or_panic("git+http");
    #[cfg(all(feature = "filesystem", feature = "networking"))]
    pub const SCHEME_GIT_HTTPS: &Scheme = Scheme::new_or_panic("git+https");
    #[cfg(all(feature = "filesystem", feature = "networking"))]
    pub const SCHEME_HTTP: &Scheme = Scheme::new_or_panic("http");
    #[cfg(all(feature = "filesystem", feature = "networking"))]
    pub const SCHEME_HTTPS: &Scheme = Scheme::new_or_panic("https");
}

pub(crate) fn format_sources(mut error: &dyn Error) -> String {
    let mut message = error.to_string();
    while let Some(source) = error.source() {
        writeln!(&mut message, "  caused by: {source}").unwrap();
        error = source;
    }
    message
}
