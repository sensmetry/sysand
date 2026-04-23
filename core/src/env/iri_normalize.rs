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

/// Canonicalize `iri` into the byte sequence that will be SHA-256'd to form
/// the `_iri/<hash>` bucket name. Returns an error if the host fails IDN
/// conversion.
///
/// The caller is responsible for parsing the IRI up front: accepting an
/// already-parsed `Iri` means no stage in the pipeline re-parses its input.
pub(crate) fn normalize_iri_for_hash(iri: &Iri<&str>) -> Result<String, IriNormalizeError> {
    let normalized = iri.normalize();
    let with_idn = punycode_host(&normalized)?;
    Ok(ensure_http_root_path(&normalized, with_idn))
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

/// For `http` / `https` IRIs with an empty path, insert `/` before any query
/// or fragment. `fluent_uri::normalize` deliberately leaves the empty path
/// untouched; WHATWG URL serialization produces the slash. Scheme and path
/// are read from `iri` (a preceding host rewrite does not affect either), so
/// no re-parse of `s` is needed.
fn ensure_http_root_path(iri: &Iri<String>, s: String) -> String {
    let scheme = iri.scheme();
    let is_http = scheme == SCHEME_HTTP || scheme == SCHEME_HTTPS;
    if !is_http || !iri.path().as_str().is_empty() {
        return s;
    }
    match s.find(['?', '#']) {
        Some(i) => format!("{}/{}", &s[..i], &s[i..]),
        None => format!("{s}/"),
    }
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
