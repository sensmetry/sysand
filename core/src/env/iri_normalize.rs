// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! IRI canonicalization for the `_iri/<hash>` bucket.
//!
//! Non-`pkg:sysand` IRIs are located under `_iri/<sha256_hex(normalized_iri)>`,
//! so two clients pointing at the same project must produce byte-identical
//! input to the hash. The canonicalization pipeline is intentionally thin glue
//! over well-known libraries — all substantive work is delegated:
//!
//! 1. [`fluent_uri::Iri::normalize`] — RFC 3986 §6.2.2 + RFC 3987 §5.3.2
//!    syntax-based normalization plus IPv6 canonicalization (RFC 5952) and
//!    scheme-default port stripping.
//! 2. [`idna::domain_to_ascii`] — WHATWG URL `domainToASCII` for any RegName
//!    host containing non-ASCII characters.
//! 3. A one-line fixup that sets the path to `/` when the scheme is
//!    `http`/`https` and the parsed path is empty, matching WHATWG URL
//!    serialization.

use fluent_uri::{Iri, ParseError, component::Host};

use crate::resolve::reqwest_http::{SCHEME_HTTP, SCHEME_HTTPS};

/// Canonicalize `iri` into the form that will be SHA-256'd to build the
/// `_iri/<hash>` bucket name. Returns the canonicalized serialization as
/// a `String`; returns an error if the host fails IDN conversion.
///
/// `iri` is taken by value: `Iri<&str>` wraps a `&str` so the move is
/// essentially free, and no caller reuses the parsed form after
/// handing it in.
pub(crate) fn normalize_iri_for_hash(iri: Iri<&str>) -> Result<String, IriNormalizeError> {
    let normalized = iri.normalize();
    let with_idn = punycode_host(&normalized)?;

    // For `http`/`https` with an empty path, WHATWG URL serialization
    // produces a `/` before any query/fragment; `fluent_uri::normalize`
    // deliberately leaves the path untouched, so apply the fixup here.
    // Scheme and path are read from `normalized` because `punycode_host`
    // only edits the host — scheme and path are bytewise identical in
    // `normalized` and `with_idn`, so reading them from either is fine.
    let scheme = normalized.scheme();
    let needs_root_slash =
        (scheme == SCHEME_HTTP || scheme == SCHEME_HTTPS) && normalized.path().as_str().is_empty();
    let final_string = if needs_root_slash {
        match with_idn.find(['?', '#']) {
            Some(i) => format!("{}/{}", &with_idn[..i], &with_idn[i..]),
            None => format!("{with_idn}/"),
        }
    } else {
        with_idn
    };

    // The three pipeline stages each preserve RFC 3987 IRI validity:
    // `Iri::normalize` is a syntax-based rewrite, `domain_to_ascii`
    // replaces only the RegName host with an ASCII Punycode label, and
    // the HTTP-root fixup only inserts `/` into an already structurally
    // valid IRI. Re-parsing is therefore infallible by construction;
    // the debug-assert guards against a regression in any of those stages.
    debug_assert!(
        Iri::parse(final_string.as_str()).is_ok(),
        "normalization pipeline output is RFC 3987 IRI-valid by construction"
    );
    Ok(final_string)
}

/// Replace a non-ASCII RegName host with its `domainToASCII` (Punycode) form.
/// IPv4, IPv6 literals, and already-ASCII RegNames pass through untouched.
/// Returns the resulting serialization as an owned `String`; the rewrite is a
/// localized splice on a known-valid IRI and does not rebuild via the IRI
/// builder (whose strict typestate is awkward for "change only the host").
fn punycode_host(iri: &Iri<String>) -> Result<String, IriNormalizeError> {
    let s = iri.as_str();
    let Some(authority) = iri.authority() else {
        return Ok(s.to_owned());
    };
    let raw_host = authority.host();
    let needs_idn = matches!(authority.host_parsed(), Host::RegName(_)) && !raw_host.is_ascii();
    if !needs_idn {
        return Ok(s.to_owned());
    }
    let ascii_host =
        idna::domain_to_ascii(raw_host).map_err(|_| IriNormalizeError::IdnConversion {
            host: raw_host.to_owned(),
        })?;
    let host_start = raw_host.as_ptr() as usize - s.as_ptr() as usize;
    let host_end = host_start + raw_host.len();
    Ok(format!(
        "{}{}{}",
        &s[..host_start],
        ascii_host,
        &s[host_end..]
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum IriNormalizeError {
    #[error("IRI is not a well-formed RFC 3987 IRI: {0}")]
    Parse(ParseError),
    #[error("host `{host}` is not a valid IDN and cannot be converted to Punycode")]
    IdnConversion { host: String },
}

#[cfg(test)]
#[path = "./iri_normalize_tests.rs"]
mod tests;
