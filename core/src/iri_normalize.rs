// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{char::REPLACEMENT_CHARACTER, fmt::Write as _};

use crate::purl::parse_sysand_purl;
use crate::utils::scheme::{SCHEME_HTTP, SCHEME_HTTPS};
use fluent_uri::{
    Iri,
    component::Host,
    pct_enc::{self, DecodedChunk, EStr},
};
use icu_casemap::CaseMapperBorrowed;
use icu_normalizer::ComposingNormalizerBorrowed;
use icu_properties::{
    CodePointSetDataBorrowed,
    props::{BidiControl, DefaultIgnorableCodePoint},
};
use idna::punycode;

// TODO: use IRI canonicalization to deduplicate usages:
// - canonicalize each IRI before trying to resolve
// - canonicalize each usage before adding
// - use canonical versions in `sysand-lock.toml` and `env.toml` (for identifiers
//   and usages)

/// Canonicalize IRI. Currently only used for the `_iri/<hash>` bucket.
///
/// Non-`pkg:sysand` IRIs are located under `_iri/<sha256_hex(normalized_iri)>`,
/// so two clients pointing at the same project must produce byte-identical
/// input to the hash. The canonicalization pipeline is intentionally thin glue
/// over well-known libraries — all substantive work is delegated:
///
/// 1. [`fluent_uri::Iri::normalize`] — RFC 3986 §6.2.2 + RFC 3987 §5.3.2
///    syntax-based normalization plus IPv6 canonicalization (RFC 5952) and
///    scheme-default port stripping.
/// 2. [`idna::domain_to_ascii`] — WHATWG URL `domainToASCII` for any RegName
///    host containing non-ASCII characters.
/// 3. A one-line fixup that sets the path to `/` when the scheme is
///    `http`/`https` and the parsed path is empty, matching WHATWG URL
///    serialization.
///
/// Returns the canonicalized serialization as
/// a `String`, or an error if the host fails IDN conversion.
pub(crate) fn canonicalize_iri(iri: Iri<&str>) -> Result<String, IriNormalizeError> {
    let normalized = iri.normalize();
    let with_idn = punycode_host(&normalized)?;

    // For `http`/`https` with an empty path, WHATWG URL serialization
    // produces a `/` before any query/fragment; `fluent_uri::normalize`
    // deliberately leaves the path untouched, so apply the fixup here.
    // Scheme and path are read from `normalized` because `punycode_host`
    // only edits the host.
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

    debug_assert!(
        Iri::parse(final_string.as_str()).is_ok(),
        "canonical IRI must remain valid"
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
    Parse(fluent_uri::ParseError),
    #[error("host `{host}` is not a valid IDN and cannot be converted to Punycode")]
    IdnConversion { host: String },
}

const CASE_MAPPER: CaseMapperBorrowed = CaseMapperBorrowed::new();
const NFC_NORMALIZER: ComposingNormalizerBorrowed = ComposingNormalizerBorrowed::new_nfc();
const BIDI_CONTROL: CodePointSetDataBorrowed = CodePointSetDataBorrowed::new::<BidiControl>();
const IGNORABLE: CodePointSetDataBorrowed =
    CodePointSetDataBorrowed::new::<DefaultIgnorableCodePoint>();

const MAX_VERSION_LEN_BYTES: u8 = 30;
const MAX_IRI_LEN_BYTES: u8 = 120;

/// Make a string from IRI that can be used as (part of) a filename in any OS
/// without risk of being invalid or potentially clasing with other names due
/// to case- or normalization-insensitivity.
///
/// A precondition for `iri` is that it must not have a fragment. Fragment presence
/// would not affect output, but would be ggod to enforce, as we currently don't
/// do anything with fragments.
/// The final form will be of the shape `host-path-query`.
fn iri_to_filename_part(iri: Iri<&str>) -> String {
    debug_assert!(!iri.has_fragment());

    let mut result = String::new();
    // Make IRI canonical.
    // We don't need canonicalization to punycode and trailing slash,
    // as we'll decode punycode and strip the trailing slash.
    // So `canonicalize_iri` is redundant
    let canonical = iri.normalize();

    // Special case PURL of the form: `pkg:sysand/<publisher>/<name>`
    if let Ok(Some((publisher, name))) = parse_sysand_purl(canonical.as_str()) {
        result.push_str(publisher);
        result.push('-');
        result.push_str(name);
        return result;
    }
    // Handle authority, leaving only (punycode-decoded) host
    if let Some(domain) = canonical.authority() {
        // TODO: which schemes use Punycode?
        let host = domain.host();
        // Try decoding Punycode
        if !host.is_empty() {
            let mut component_it = host.split('.').peekable();
            if component_it.peek().is_some() {
                for component in component_it {
                    if let Some(c) = component.strip_prefix("xn--") {
                        // TODO: maybe use https://docs.rs/idna/latest/idna/uts46/struct.Uts46.html#method.to_user_interface
                        if let Some(puny_decoded) = punycode::decode_to_string(c) {
                            result.push_str(&puny_decoded);
                        } else {
                            result.push_str(component);
                        }
                    } else {
                        result.push_str(component);
                    }
                    result.push('.');
                }
                result.pop();
            } else {
                // Domain is a single hostname
                if let Some(c) = host.strip_prefix("xn--") {
                    if let Some(puny_decoded) = punycode::decode_to_string(c) {
                        result.push_str(&puny_decoded);
                    } else {
                        result.push_str(host);
                    }
                } else {
                    result.push_str(host);
                }
            }
            result.push('-');
        }
    }

    // Decode percent-encoded octets and strip out bytes that don't form valid
    // UTF-8 sequences
    extend_decode_strip_invalid(&mut result, canonical.path());
    // Strip `.kpar` suffix to reduce length
    if result.ends_with(".kpar") {
        result.truncate(result.len() - 5);
    }
    if let Some(query) = canonical.query() {
        result.push('-');
        extend_decode_strip_invalid(&mut result, query);
    }

    // Strip all beginning/ending non-alphanumeric ASCII and spacing/control
    // non-ASCII characters.
    // Rationale: prevent problems with Windows, do not make files hidden on Unix. Also
    // avoid problems with invisible characters.

    result = result
        .trim_matches(|c: char| {
            (c.is_ascii() && !c.is_ascii_alphanumeric())
                || c.is_control()
                || c.is_whitespace()
                || IGNORABLE.contains(c)
        })
        .to_owned();

    // Replace invalid/undesirable ASCII punctuation/whitespace:
    // - any amount of whitespace with any surrounding non-alphanumeric ASCII chars
    //   with a single `_`
    // - `/` or `:` with `.`. Having a `.` requires considering Windows "special paths"
    //   like `CON`, `LPT` and such - cannot have names that start with these bare or if
    //   they are followed by a `.` followed by anything.
    // - any amount of illegal ASCII characters or multiple punctuation symbols
    //   with a single `-`
    // Note that since Unicode (non-ASCII) punctuation/whitespace has no special
    // treatment in shells/OSs (at least common ones), it's left as-is, except for
    // bidi controls, as these if unterminated create a not-well-formed string
    let mut c_it = result.chars().peekable();
    let mut sanitized = String::new();
    while let Some(c) = c_it.next() {
        if c.is_whitespace() {
            // Discard previous ASCII non-alphanum
            while let Some(previous) = sanitized.chars().last() {
                if previous.is_ascii() && !previous.is_ascii_alphanumeric() {
                    sanitized.pop();
                } else {
                    break;
                }
            }
            // Discard subsequent ASCII non-alphanum
            while c_it
                .next_if(|next| next.is_ascii() && !next.is_ascii_alphanumeric())
                .is_some()
            {}
            sanitized.push('_');
        } else if c == '/' || c == ':' {
            sanitized.push('.');
        } else if c.is_ascii() && !c.is_ascii_alphanumeric() {
            let mut discarded = 0u32;
            // Discard previous ASCII non-alphanum
            while let Some(previous) = sanitized.chars().last() {
                if previous.is_ascii() && !previous.is_ascii_alphanumeric() {
                    sanitized.pop();
                    discarded += 1;
                } else {
                    break;
                }
            }
            // Discard subsequent ASCII non-alphanum
            while c_it
                .next_if(|next| next.is_ascii() && !next.is_ascii_alphanumeric())
                .is_some()
            {
                discarded += 1;
            }
            if discarded == 0 && (c == '-' || c == '.' || c == '_') {
                sanitized.push(c);
            } else {
                sanitized.push('-');
            }
        // Windows does not work well with U+FFFD (replacement character) in file name.
        // IRIs can't contain U+FFFD literally, but it can be present if percent-encoded
        } else if !BIDI_CONTROL.contains(c) && c != REPLACEMENT_CHARACTER {
            sanitized.push(c);
        } else if !sanitized.ends_with('.') && !sanitized.ends_with('-') {
            sanitized.push('-');
        }
    }

    // Apply full Unicode case folding.
    // Rationale: macOS/Windows and some Linux configurations are case-insensitive
    let casefolded = CASE_MAPPER.fold_string(&sanitized);

    // Normalize using Unicode NFC.
    // Rationale: macOS is normalization-insensitive
    let nfc = NFC_NORMALIZER.normalize(&casefolded);
    // Truncate to 120 bytes. If ending bytes now form an invalid UTF-8 char, remove them
    let truncation_boundary = nfc.floor_char_boundary(MAX_IRI_LEN_BYTES.into());
    let truncated = &nfc[..truncation_boundary];

    // Strip all ending non-alphanumeric ASCII and spacing/control
    // non-ASCII characters.
    truncated
        .trim_end_matches(|c: char| {
            (c.is_ascii() && !c.is_ascii_alphanumeric())
                || c.is_control()
                || c.is_whitespace()
                || IGNORABLE.contains(c)
        })
        .to_owned()

    // Note: Windows special file names, like CON, LPT, etc. don't need to be handled
    // here, since file names will always have version appended. Only CON and
    // CON.<extension> are reserved, and version always contains dots, so even if
    // IRI normalizes to CON or CON.txt, the "extension" will be part after the last dot,
    // which will be in the version
}

/// Decode `src`, which may contain percent-encoded bytes. Strip byte sequences
/// that do not form valid UTF-8 chars
/// This matches `String::from_utf8_lossy`, but instead of replacing
/// invalid sequences with U+FFFD, it drops them
fn extend_decode_strip_invalid(result: &mut String, src: &EStr<impl pct_enc::Encoder>) {
    // This stores 0..=4 percent-decoded bytes
    let mut bytes = [0; 4];
    let mut bytes_used = 0;
    for dec in src.decode() {
        match dec {
            DecodedChunk::Unencoded(s) => {
                result.push_str(s);
                // Ignore possible previous incomplete char
                bytes_used = 0;
            }
            DecodedChunk::PctDecoded(b) => {
                bytes[bytes_used] = b;
                bytes_used += 1;
                match str::from_utf8(&bytes[0..bytes_used]) {
                    Ok(s) => {
                        result.push_str(s);
                        bytes_used = 0;
                    }
                    Err(e) => {
                        // If this is `None`, the sequence is an incomplete possible char,
                        // wait for more bytes
                        if e.error_len().is_some() {
                            // The sequence is invalid, but the last byte might be valid
                            // (start of) sequence
                            bytes[0] = b;
                            bytes_used = 1;
                            match str::from_utf8(&bytes[0..bytes_used]) {
                                Ok(s) => {
                                    result.push_str(s);
                                    bytes_used = 0;
                                }
                                Err(e) => {
                                    if e.error_len().is_some() {
                                        // Byte is invalid
                                        bytes_used = 0;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Normalize version to be used in a filename.
/// Supports arbitrary version strings.
pub fn normalize_version<V: AsRef<str>>(version: V) -> String {
    // Strip all beginning/ending non-alphanumeric ASCII and spacing/control
    // non-ASCII characters.
    let trimmed = version.as_ref().trim_matches(|c: char| {
        (c.is_ascii() && !c.is_ascii_alphanumeric())
            || c.is_control()
            || c.is_whitespace()
            || IGNORABLE.contains(c)
    });

    // Apply full Unicode case folding.
    let casefolded = CASE_MAPPER.fold_string(trimmed);
    // Normalize using Unicode NFC.
    let nfc = NFC_NORMALIZER.normalize(&casefolded);

    // Replace any ASCII char not in `a-zA-Z0-9.-` with `-`.
    let mut sanitized = String::with_capacity(nfc.len());
    for c in nfc.chars() {
        if c.is_ascii() {
            if c.is_ascii_alphanumeric() {
                sanitized.push(c);
            } else if !sanitized.ends_with('.') && !sanitized.ends_with('-') {
                if c == '.' {
                    sanitized.push('.');
                } else {
                    sanitized.push('-');
                }
            }
        // Windows does not work well with U+FFFD (replacement character) in file name
        } else if !BIDI_CONTROL.contains(c) && c != REPLACEMENT_CHARACTER {
            sanitized.push(c);
        } else if !sanitized.ends_with('.') && !sanitized.ends_with('-') {
            sanitized.push('-');
        }
    }

    // Truncate the version string to 30 bytes. Remove ending bytes if they form an
    // invalid UTF-8 char.
    let truncation_boundary = sanitized.floor_char_boundary(MAX_VERSION_LEN_BYTES.into());
    let truncated = &sanitized[..truncation_boundary];

    // Strip all ending non-alphanumeric ASCII and spacing/control
    // non-ASCII characters.
    truncated
        .trim_end_matches(|c: char| {
            (c.is_ascii() && !c.is_ascii_alphanumeric())
                || c.is_control()
                || c.is_whitespace()
                || IGNORABLE.contains(c)
        })
        .to_owned()
}

/// Generates filename candidates from a given IRI and version.
/// Format: `<normalized_iri>[_<disambiguation_number>]_<normalized_version>`
#[derive(Debug)]
pub struct IriVersionFilename {
    filename: String,
    version: String,
    iri_end_idx: u32,
    disambiguation_number: u32,
}

impl IriVersionFilename {
    pub fn new<V: AsRef<str>>(iri: Iri<&str>, version: V) -> Self {
        let mut normalized_iri = iri_to_filename_part(iri);
        let normalized_version = normalize_version(version);

        normalized_iri.push('_');
        let iri_end_idx = normalized_iri.len() as u32;

        Self {
            filename: normalized_iri,
            version: normalized_version,
            iri_end_idx,
            disambiguation_number: 0,
        }
    }

    /// Produce a candidate filename. On first call this will produce a name
    /// without a disambiguation number, on the next call a disambiguation
    /// number will be added and on later calls incremented.
    pub fn next_candidate(&mut self) -> &str {
        match self.disambiguation_number {
            0 => self.filename.push_str(&self.version),
            n => {
                self.filename.truncate(self.iri_end_idx as usize);
                write!(self.filename, "{n}_{}", self.version).unwrap();
            }
        }
        self.disambiguation_number += 1;
        &self.filename
    }
}

impl From<IriVersionFilename> for String {
    /// Use this only after ensuring that the filename is unique
    fn from(value: IriVersionFilename) -> Self {
        value.filename
    }
}

#[cfg(test)]
#[path = "./iri_normalize_tests.rs"]
mod tests;
