// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! IRI canonicalization for the `_iri/<hash>` bucket.
//!
//! Non-`pkg:sysand` IRIs are located under `_iri/<sha256_hex(normalized_iri)>`,
//! so two clients pointing at the same project must produce byte-identical
//! input to the hash. The canonicalization pipeline is intentionally thin glue
//! over well-known libraries — all substantive work is delegated:
//!
//! 1. [`fluent_uri::IriRef::normalize`] — RFC 3986 §6.2.2 + RFC 3987 §5.3.2
//!    syntax-based normalization plus IPv6 canonicalization (RFC 5952) and
//!    scheme-default port stripping.
//! 2. [`idna::domain_to_ascii`] — WHATWG URL `domainToASCII` for any RegName
//!    host containing non-ASCII characters.
//! 3. A one-line fixup that sets the path to `/` when the scheme is
//!    `http`/`https` and the parsed path is empty, matching WHATWG URL
//!    serialization.

use fluent_uri::{IriRef, ParseError, component::Host};

/// Canonicalize `iri` into the byte sequence that will be SHA-256'd to form
/// the `_iri/<hash>` bucket name. Returns an error if the input is not a
/// well-formed IRI reference or if the host fails IDN conversion.
pub(crate) fn normalize_iri_for_hash(iri: &str) -> Result<String, IriNormalizeError> {
    let parsed = IriRef::parse(iri).map_err(IriNormalizeError::Parse)?;
    let normalized = parsed.normalize();
    let with_idn = punycode_host(normalized.as_str())?;
    Ok(ensure_http_root_path(&with_idn))
}

/// Replace a non-ASCII RegName host with its `domainToASCII` (Punycode) form.
/// IPv4, IPv6 literals, and already-ASCII RegNames pass through untouched.
fn punycode_host(s: &str) -> Result<String, IriNormalizeError> {
    let parsed = IriRef::parse(s).expect("output of normalize() must re-parse as IriRef");
    let Some(authority) = parsed.authority() else {
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
/// untouched; WHATWG URL serialization produces the slash.
fn ensure_http_root_path(s: &str) -> String {
    let Ok(parsed) = IriRef::parse(s) else {
        return s.to_owned();
    };
    let is_http_scheme = matches!(
        parsed.scheme().map(|sc| sc.as_str()),
        Some("http") | Some("https")
    );
    if !is_http_scheme || !parsed.path().as_str().is_empty() {
        return s.to_owned();
    }
    match s.find(['?', '#']) {
        Some(i) => format!("{}/{}", &s[..i], &s[i..]),
        None => format!("{s}/"),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IriNormalizeError {
    #[error("IRI is not a well-formed RFC 3987 IRI reference: {0}")]
    Parse(ParseError),
    #[error("host `{host}` is not a valid IDN and cannot be converted to Punycode")]
    IdnConversion { host: String },
}

#[cfg(test)]
#[path = "./iri_normalize_tests.rs"]
mod tests;
