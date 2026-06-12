// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{error::Error, fmt::Write as _};

use digest::{array::Array, typenum};
use indexmap::IndexSet;
use sha2::{Digest, Sha256};

pub(crate) mod scheme {
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
    pub const SCHEME_HTTP: &Scheme = Scheme::new_or_panic("http");
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

pub(crate) fn multiline_array(
    elements: impl Iterator<Item = impl Into<toml_edit::Value>>,
) -> toml_edit::Array {
    let mut array: toml_edit::Array = elements
        .map(|item| {
            let mut value = item.into();
            value.decor_mut().set_prefix("\n    ");
            value
        })
        .collect();
    array.set_trailing_comma(true);
    array.set_trailing("\n");
    array
}

pub fn sha256_lowercase_hex(data: impl AsRef<[u8]>) -> String {
    lowercase_hex(Sha256::digest(data))
}

/// Encode `bytes` as lowercase hex string
pub fn lowercase_hex(bytes: Array<u8, typenum::U32>) -> String {
    hex::encode(bytes)
}

/// Return the deduplicated, in-order list of SPDX identifiers (licenses plus
/// any `WITH` exceptions) named in `expression`. Each identifier maps to a
/// `LICENSES/<id>.txt` file under REUSE conventions; the `+` "or later"
/// modifier does not affect the filename.
pub(crate) fn license_file_stems(expression: &spdx::Expression) -> IndexSet<String> {
    let mut stems: IndexSet<String> = IndexSet::new();
    for req in expression.requirements() {
        let license_name = match &req.req.license {
            spdx::LicenseItem::Spdx { id, .. } => id.name.to_string(),
            spdx::LicenseItem::Other(license_ref) => license_ref.to_string(),
        };
        stems.insert(license_name);

        if let Some(addition) = &req.req.addition {
            let addition_name = match addition {
                spdx::AdditionItem::Spdx(id) => id.name.to_string(),
                spdx::AdditionItem::Other(add_ref) => add_ref.to_string(),
            };
            stems.insert(addition_name);
        }
    }
    stems
}
