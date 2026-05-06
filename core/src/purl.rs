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
/// this prefix, both satisfying the field rules enforced by
/// [`is_valid_publisher`] and [`is_valid_name`].
pub const PKG_SYSAND_PREFIX: &str = "pkg:sysand/";

/// Which kind of `pkg:sysand` segment to validate. Publishers disallow dots,
/// but names permit them so that dotted product names (e.g.
/// `foo.bar`) are expressible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldKind {
    Publisher,
    Name,
}

impl FieldKind {
    fn allows_dot_separator(self) -> bool {
        self == FieldKind::Name
    }
}

/// Validates a publisher or name field for `pkg:sysand` project IDs.
///
/// Rules: 3-50 ASCII alphanumeric characters, with single separators (space,
/// hyphen, and — for names — dot) allowed between words. Must
/// start and end with an alphanumeric character.
fn is_valid_unnormalized_field(s: &str, kind: FieldKind) -> bool {
    is_valid_sysand_purl_part(&normalize_field(s), kind)
}

/// Whether `s` can, after normalization, be used as Sysand PURL publisher.
pub fn is_valid_unnormalized_publisher(s: &str) -> bool {
    is_valid_unnormalized_field(s, FieldKind::Publisher)
}

/// Whether `s` can, after normalization, be used as Sysand PURL name.
pub fn is_valid_unnormalized_name(s: &str) -> bool {
    is_valid_unnormalized_field(s, FieldKind::Name)
}

/// Validates a publisher or name field for `pkg:sysand` project IDs.
///
/// Rules: 3-50 ASCII alphanumeric characters, with single separators (space,
/// hyphen, and — for names — dot) allowed between words. Must
/// start and end with an alphanumeric character.
fn is_valid_sysand_purl_part(s: &str, kind: FieldKind) -> bool {
    let is_lower_or_digit = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let bytes = s.as_bytes();

    if !(3..=50).contains(&bytes.len()) {
        return false;
    }

    if !is_lower_or_digit(bytes[0]) || !is_lower_or_digit(bytes[bytes.len() - 1]) {
        return false;
    }

    for &[b_previous, b] in bytes[..bytes.len() - 1].array_windows() {
        if is_lower_or_digit(b) {
            continue;
        }

        let is_separator = b == b'-' || (kind.allows_dot_separator() && b == b'.');
        // This will also catch all non-ASCII
        if !is_separator {
            return false;
        }

        // only isolated separators — knowing first/last is alphanumeric,
        // this is sufficient
        if !is_lower_or_digit(b_previous) {
            return false;
        }
    }

    true
}

/// Whether `s` is a valid Sysand PURL publisher segment.
pub fn is_valid_purl_publisher(s: &str) -> bool {
    is_valid_sysand_purl_part(s, FieldKind::Publisher)
}

/// Whether `s` is a valid Sysand PURL project name segment.
pub fn is_valid_purl_name(s: &str) -> bool {
    is_valid_sysand_purl_part(s, FieldKind::Name)
}

/// Canonicalizes a publisher or name by lowercasing ASCII and replacing spaces
/// with hyphens. The result is what ends up embedded in a `pkg:sysand` IRI;
/// callers should validate with [`is_valid_publisher`] or [`is_valid_name`]
/// before or after calling.
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
    #[error(
        "`{purl}` is not a valid `pkg:sysand` PURL: expected exactly two \
         `/`-separated segments after `pkg:sysand/`, found {segments}"
    )]
    WrongShape { purl: String, segments: usize },
    /// The publisher segment did not satisfy the `pkg:sysand` field rules
    /// (length, allowed characters, separator placement).
    #[error(
        "`{purl}` is not a valid `pkg:sysand` PURL: publisher segment `{publisher}` \
         is not a valid `pkg:sysand` field \
         (3-50 ASCII alphanumeric chars, with single ` ` or `-` separators between words)"
    )]
    InvalidPublisher { purl: String, publisher: String },
    /// The name segment did not satisfy the `pkg:sysand` field rules.
    #[error(
        "`{purl}` is not a valid `pkg:sysand` PURL: name segment `{name}` \
         is not a valid `pkg:sysand` field \
         (3-50 ASCII alphanumeric chars, with single ` `, `-`, or `.` separators between words)"
    )]
    InvalidName { purl: String, name: String },
    /// Both segments are valid as unnormalized, but at least one is not in normalized form
    #[error(
        "`{purl}` is not a valid `pkg:sysand` PURL, but can be normalized to \
         `pkg:sysand/{norm_publisher}/{norm_name}`"
    )]
    NotNormalized {
        purl: String,
        norm_publisher: String,
        norm_name: String,
    },
}

/// Parse a `pkg:sysand/<publisher>/<name>` IRI into its `(publisher, name)`
/// segments. Returns `Ok(None)` for IRIs that do not start with the
/// `pkg:sysand/` prefix, `Ok(Some(..))` for conforming Sysand PURLs,
/// and `Err(_)` for IRIs that start with the prefix but fail validation.
pub fn parse_sysand_purl(iri: &str) -> Result<Option<(&str, &str)>, SysandPurlError> {
    // scheme is case-insensitive
    let rest = if iri.len() >= PKG_SYSAND_PREFIX.len()
        && iri.as_bytes()[0..3].eq_ignore_ascii_case(b"pkg")
        && iri.as_bytes()[3..].starts_with(b":sysand/")
    {
        iri.split_at(PKG_SYSAND_PREFIX.len()).1
    } else {
        return Ok(None);
    };

    let parts: Vec<&str> = rest.split('/').collect();
    let [publisher, name] = parts.as_slice() else {
        return Err(SysandPurlError::WrongShape {
            purl: iri.to_owned(),
            segments: parts.len(),
        });
    };

    if !is_valid_purl_name(name) || !is_valid_purl_publisher(publisher) {
        if is_valid_unnormalized_name(name) && is_valid_unnormalized_publisher(publisher) {
            return Err(SysandPurlError::NotNormalized {
                purl: iri.to_owned(),
                norm_publisher: normalize_field(publisher),
                norm_name: normalize_field(name),
            });
        }

        if !is_valid_purl_publisher(publisher) {
            return Err(SysandPurlError::InvalidPublisher {
                purl: iri.to_owned(),
                publisher: (*publisher).to_owned(),
            });
        }

        return Err(SysandPurlError::InvalidName {
            purl: iri.to_owned(),
            name: (*name).to_owned(),
        });
    }

    Ok(Some((publisher, name)))
}

#[cfg(test)]
#[path = "./purl_tests.rs"]
mod tests;
