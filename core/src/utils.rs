// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt::Write as _,
};

use digest::{array::Array, typenum};
use icu_casemap::CaseMapperBorrowed;
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use icu_properties::{
    CodePointMapDataBorrowed, CodePointSetDataBorrowed,
    props::{BidiControl, DefaultIgnorableCodePoint, GeneralCategory, XidContinue},
};
use indexmap::IndexSet;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::project::{memory::InMemoryProject, utils::Identifier};

pub type ProvidedProjects = HashMap<Identifier, Vec<InMemoryProject>>;
pub type ProvidedIdentifiers = HashSet<Identifier>;

// Normalization, case folding and character sets, shared by everything that
// validates or canonicalizes text

/// Full Unicode case folding
pub(crate) const CASE_MAPPER: CaseMapperBorrowed = CaseMapperBorrowed::new();
pub(crate) const NFC_NORMALIZER: ComposingNormalizerBorrowed =
    ComposingNormalizerBorrowed::new_nfc();
pub(crate) const NFKC_NORMALIZER: ComposingNormalizerBorrowed =
    ComposingNormalizerBorrowed::new_nfkc();
pub(crate) const NFKD_NORMALIZER: DecomposingNormalizerBorrowed =
    DecomposingNormalizerBorrowed::new_nfkd();
/// `Bidi_Control` characters are invisible directional formatting
pub(crate) const BIDI_CONTROL: CodePointSetDataBorrowed =
    CodePointSetDataBorrowed::new::<BidiControl>();
/// `Default_Ignorable_Code_Point` characters are normally invisible
pub(crate) const IGNORABLE: CodePointSetDataBorrowed =
    CodePointSetDataBorrowed::new::<DefaultIgnorableCodePoint>();
/// Unicode identifier continuation (UAX #31)
pub(crate) const XID_CONTINUE: CodePointSetDataBorrowed =
    CodePointSetDataBorrowed::new::<XidContinue>();
/// Character Unicode category
pub(crate) const GENERAL_CATEGORY: CodePointMapDataBorrowed<GeneralCategory> =
    CodePointMapDataBorrowed::new();

/// ASCII punctuation allowed in publisher/name in addition to identifier
/// chars, to allow common organization names (e.g. `ACME Inc.`, `AT&T`,
/// `O'Reilly`, `Foo, Inc.`, `C++ Tools`, `Foo (EU)`). Other ASCII
/// punctuation is excluded, as it needs quoting or escaping in shells, JSON
/// or TOML. Non-ASCII punctuation is allowed without restrictions
pub(crate) const PROJECT_FIELD_ASCII_PUNCTUATION: [char; 10] =
    [' ', '-', '_', '.', '&', '\'', ',', '+', '(', ')'];

/// The separator between the words of a `pkg:sysand` PURL publisher or name
pub(crate) const PURL_SEPARATOR: u8 = b'-';
/// The separator between the words of a `pkg:sysand` PURL name, in addition
/// to [`PURL_SEPARATOR`]
pub(crate) const PURL_NAME_SEPARATOR: u8 = b'.';
/// The separator between the words of a not yet normalized `pkg:sysand` PURL
/// publisher or name, in addition to [`PURL_SEPARATOR`]; normalization turns
/// it into [`PURL_SEPARATOR`]
pub(crate) const UNNORMALIZED_PURL_SEPARATOR: u8 = b' ';

#[cfg(feature = "filesystem")]
pub(crate) mod scheme {
    use fluent_uri::component::Scheme;
    pub const SCHEME_FILE: &Scheme = Scheme::new_or_panic("file");
    #[cfg(feature = "networking")]
    pub const SCHEME_SSH: &Scheme = Scheme::new_or_panic("ssh");
    #[cfg(feature = "networking")]
    pub const SCHEME_GIT_SSH: &Scheme = Scheme::new_or_panic("git+ssh");
    #[cfg(feature = "networking")]
    pub const SCHEME_GIT_FILE: &Scheme = Scheme::new_or_panic("git+file");
    #[cfg(feature = "networking")]
    pub const SCHEME_GIT_HTTP: &Scheme = Scheme::new_or_panic("git+http");
    #[cfg(feature = "networking")]
    pub const SCHEME_GIT_HTTPS: &Scheme = Scheme::new_or_panic("git+https");
    #[cfg(feature = "filesystem")]
    pub const SCHEME_HTTP: &Scheme = Scheme::new_or_panic("http");
    #[cfg(feature = "filesystem")]
    pub const SCHEME_HTTPS: &Scheme = Scheme::new_or_panic("https");
}

/// Format an error, together with a chain of its `source()`.
/// Should be used in all cases where an error is turned into
/// a string and `source()` is not checked
pub fn format_err(error: impl Error) -> String {
    let mut error: &dyn Error = &error;
    let mut message = error.to_string();
    while let Some(source) = error.source() {
        write!(&mut message, "\n  caused by: {source}").unwrap();
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
            spdx::LicenseItem::Spdx { id, or_later: _ } => id.name.to_owned(),
            spdx::LicenseItem::Other(license_ref) => license_ref.to_string(),
        };
        stems.insert(license_name);

        if let Some(addition) = &req.req.addition {
            let addition_name = match addition {
                spdx::AdditionItem::Spdx(id) => id.name.to_owned(),
                spdx::AdditionItem::Other(add_ref) => add_ref.to_string(),
            };
            stems.insert(addition_name);
        }
    }
    stems
}

pub fn to_pretty_json_string<T: Serialize>(value: &T) -> String {
    // If this fails, it's a bug
    let mut serialized = serde_json::to_string_pretty(value).unwrap();
    // Text files should have a trailing newline
    serialized.push('\n');
    serialized
}

pub const SP: char = ' ';

/// Render an expiry timestamp for display, without sub-second
/// noise (`11:39:28.149443` reads as `11:39:28`).
pub fn format_expiry_utc(expires_at: &jiff::Timestamp) -> String {
    expires_at.strftime("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// Whether `version`, a `major.minor` format version such as a lockfile's
/// `lock_version`, is above `current`: the mark of a file a newer release
/// wrote. Anything that is not two integers is not newer, only unknown.
pub fn is_newer_format_version(version: &str, current: &str) -> bool {
    fn parse(version: &str) -> Option<(u64, u64)> {
        let (major, minor) = version.split_once('.')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    }
    match (parse(version), parse(current)) {
        (Some(version), Some(current)) => version > current,
        _ => false,
    }
}

#[cfg(test)]
#[path = "./utils_tests.rs"]
mod tests;
