// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! PURL (Package URL) validation and normalization for `pkg:sysand` IRIs.
//!
//! Sysand uses the `pkg:sysand/<publisher>/<name>` scheme as its canonical
//! project identifier, following the [Package URL specification][purl-spec].
//! This module defines the rules that publisher and name segments must
//! satisfy and provides the normalization function that maps valid
//! human-supplied values to their canonical form.
//!
//! [purl-spec]: https://github.com/package-url/purl-spec

use thiserror::Error;

/// The `pkg:sysand/` URI scheme prefix. A `pkg:sysand` IRI is required to
/// have exactly two slash-separated segments (`<publisher>/<name>`) after
/// this prefix, both passing [`is_normalized_field`] for their respective
/// [`FieldKind`].
pub const PKG_SYSAND_PREFIX: &str = "pkg:sysand/";

/// Which kind of `pkg:sysand` segment to validate. Publishers disallow dots
/// (they would collide with reverse-DNS-shaped identifiers elsewhere in the
/// toolchain); names permit dots so that dotted product names (e.g.
/// `foo.bar`) are expressible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Publisher,
    Name,
}

impl FieldKind {
    fn dot_is_separator(self) -> bool {
        self == FieldKind::Name
    }
}

/// Validates a publisher or name field for `pkg:sysand` project IDs.
///
/// Rules: 3-50 ASCII alphanumeric characters, with single separators (space,
/// hyphen, and — for `FieldKind::Name` — dot) allowed between words. Must
/// start and end with an alphanumeric character.
pub fn is_valid_field(s: &str, kind: FieldKind) -> bool {
    if !s.is_ascii() {
        return false;
    }
    let bytes = s.as_bytes();

    if !(3..=50).contains(&bytes.len()) {
        return false;
    }

    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return false;
    }

    for i in 1..(bytes.len() - 1) {
        let b = bytes[i];

        if b.is_ascii_alphanumeric() {
            continue;
        }

        let is_separator = b == b'-' || b == b' ' || (kind.dot_is_separator() && b == b'.');
        if !is_separator {
            return false;
        }

        // only isolated separators — knowing first/last is alphanumeric,
        // this is sufficient
        if !bytes[i - 1].is_ascii_alphanumeric() {
            return false;
        }
    }

    true
}

/// Whether `s` is a valid publisher segment.
pub fn is_valid_publisher(s: &str) -> bool {
    is_valid_field(s, FieldKind::Publisher)
}

/// Whether `s` is a valid project name segment.
pub fn is_valid_name(s: &str) -> bool {
    is_valid_field(s, FieldKind::Name)
}

/// Canonicalizes a publisher or name by lowercasing ASCII and replacing spaces
/// with hyphens. The result is what ends up embedded in a `pkg:sysand` IRI;
/// callers should validate with [`is_valid_field`] before or after calling.
pub fn normalize_field(s: &str) -> String {
    s.to_ascii_lowercase().replace(' ', "-")
}

/// Reason a `pkg:sysand/...` IRI failed [`parse_sysand_purl`]. Used to
/// build human-readable validation errors that explain the rejection
/// without leaking parser internals.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SysandPurlError {
    /// Wrong number of slash-separated segments — expected exactly
    /// `<publisher>/<name>`. `segments` is the count actually seen so the
    /// error message can name it.
    #[error("expected exactly two `/`-separated segments after `pkg:sysand/`, found {segments}")]
    WrongShape { segments: usize },
    /// The publisher segment did not satisfy [`is_valid_field`] (length,
    /// allowed characters, separator placement).
    #[error(
        "publisher segment `{publisher}` is not a valid `pkg:sysand` field \
         (3-50 ASCII alphanumeric chars, with single ` ` or `-` separators between words)"
    )]
    InvalidPublisher { publisher: String },
    /// The name segment did not satisfy [`is_valid_field`].
    #[error(
        "name segment `{name}` is not a valid `pkg:sysand` field \
         (3-50 ASCII alphanumeric chars, with single ` `, `-`, or `.` separators between words)"
    )]
    InvalidName { name: String },
    /// Both segments validate, but at least one is not in normalized form
    /// (i.e. [`normalize_field`] would change it). `suggested` carries the
    /// normalized IRI so callers can show "did you mean `<x>`?".
    #[error(
        "IRI is valid but not normalized; did you mean `{suggested}` (lowercase ASCII, spaces replaced with hyphens)?"
    )]
    NotNormalized { suggested: String },
}

/// Parse a `pkg:sysand/<publisher>/<name>` IRI into its `(publisher, name)`
/// segments. Returns `Ok(None)` for IRIs that do not start with the
/// `pkg:sysand/` prefix at all (so callers can route those through a
/// different code path), `Ok(Some(..))` for well-formed and normalized IRIs,
/// and `Err(_)` for IRIs that start with the prefix but fail validation —
/// the prefix is a strong enough signal of intent that silently rerouting
/// such an IRI would mask user errors.
pub fn parse_sysand_purl(iri: &str) -> Result<Option<(&str, &str)>, SysandPurlError> {
    let Some(rest) = iri.strip_prefix(PKG_SYSAND_PREFIX) else {
        return Ok(None);
    };

    let parts: Vec<&str> = rest.split('/').collect();
    let [publisher, name] = parts.as_slice() else {
        return Err(SysandPurlError::WrongShape {
            segments: parts.len(),
        });
    };

    if !is_valid_publisher(publisher) {
        return Err(SysandPurlError::InvalidPublisher {
            publisher: (*publisher).to_owned(),
        });
    }
    if !is_valid_field(name, FieldKind::Name) {
        return Err(SysandPurlError::InvalidName {
            name: (*name).to_owned(),
        });
    }

    if normalize_field(publisher) != *publisher || normalize_field(name) != *name {
        return Err(SysandPurlError::NotNormalized {
            suggested: format!(
                "{PKG_SYSAND_PREFIX}{}/{}",
                normalize_field(publisher),
                normalize_field(name)
            ),
        });
    }

    Ok(Some((publisher, name)))
}

#[cfg(test)]
#[path = "./purl_tests.rs"]
mod tests;
