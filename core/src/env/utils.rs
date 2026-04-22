// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    char::{self, REPLACEMENT_CHARACTER},
    fmt::Write,
};

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectMut, ProjectRead,
        utils::{FsIoError, is_valid_name, is_valid_publisher},
    },
};

use borrow_or_share::Bos;
use fluent_uri::{
    Iri,
    pct_enc::{self, DecodedChunk, EStr},
};
use icu_casemap::CaseMapperBorrowed;
use icu_normalizer::ComposingNormalizerBorrowed;
use icu_properties::{
    CodePointSetDataBorrowed,
    props::{BidiControl, DefaultIgnorableCodePoint},
};
use thiserror::Error;
use url::Url;

/// Trait to use as a bound for all errors exposed through public
/// crate interfaces. This makes it convenient to use anyhow::Error.
pub trait ErrorBound: std::error::Error + Send + Sync + 'static {}
impl<T> ErrorBound for T where T: std::error::Error + Send + Sync + 'static {}

#[derive(Error, Debug)]
pub enum CloneError<ProjectReadError: ErrorBound, EnvironmentWriteError: ErrorBound> {
    #[error("project read error: {0}")]
    ProjectRead(ProjectReadError),
    #[error("environment write error: {0}")]
    EnvWrite(EnvironmentWriteError),
    #[error("incomplete project: {0}")]
    IncompleteSource(&'static str),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl<ProjectReadError: ErrorBound, EnvironmentWriteError: ErrorBound> From<FsIoError>
    for CloneError<ProjectReadError, EnvironmentWriteError>
{
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

/// Copies the project from `from` to `to`. Returns project metadata
pub fn clone_project<P: ProjectRead, Q: ProjectMut>(
    from: &P,
    to: &mut Q,
    overwrite: bool,
) -> Result<
    (InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw),
    CloneError<P::Error, Q::Error>,
> {
    match from.get_project().map_err(CloneError::ProjectRead)? {
        (None, None) => Err(CloneError::IncompleteSource(
            "missing `.project.json` and `.meta.json`",
        )),
        (None, _) => Err(CloneError::IncompleteSource("missing `.project.json`")),
        (_, None) => Err(CloneError::IncompleteSource("missing `.meta.json`")),
        (Some(info), Some(meta)) => {
            to.put_project(&info, &meta, overwrite)
                .map_err(CloneError::EnvWrite)?;

            for source_path in &meta.source_paths(true) {
                let mut source = from
                    .read_source(source_path)
                    .map_err(CloneError::ProjectRead)?;
                to.write_source(source_path, &mut source, overwrite)
                    .map_err(CloneError::EnvWrite)?;
            }
            Ok((info, meta))
        }
    }
}

// pub fn clone_project_into_unnormalised<P : ProjectRead, E : WriteEnvironment, S : AsRef<str>, T: AsRef<str>>(
//     project : &P,
//     environment : &mut E,
//     uri : S,
//     version : T,
//     overwrite : bool,
// ) -> Result<E::InterchangeProjectWrite, CloneError<P::ReadError, E::WriteError>> {
//     environment.put_project(
//         uri,
//         version,
//         |target| {
//             match project.get_project()? {
//                     (None, None) => todo!(),
//                     (None, _) => todo!(),
//                     (_, None) => todo!(),
//                     (Some(info), Some(meta)) => {
//                         target.put_project(&info, &meta, overwrite)?;
//                         Ok(())
//                     },
//                 }
//         }
//     ).map_err(|err: PutProjectError<E::WriteError, P::ReadError>| match err {

//     })
// }

// pub fn clone_project_into_normalised<P : ProjectRead, E : WriteEnvironment>(
//     project : &P,
//     environment : &mut E,
//     uri : Uri<String>,
//     version : Version,
//     overwrite : bool,
// ) -> Result<E::InterchangeProjectWrite, CloneError<P::ReadError, E::WriteError>> {
//     let nfc = icu_normalizer::ComposingNormalizerBorrowed::new_nfc();
//     let uri_str = uri.normalize();
//     let uri_normalised =
//         nfc.normalize(uri_str.as_str());

//     clone_project_into_unnormalised(
//         project,environment,
//         uri_normalised,
//         version.to_string(),
//         overwrite,
//     )
// }

const CASE_MAPPER: CaseMapperBorrowed = CaseMapperBorrowed::new();
const NFC_NORMALIZER: ComposingNormalizerBorrowed = ComposingNormalizerBorrowed::new_nfc();
const BIDI_CONTROL: CodePointSetDataBorrowed = CodePointSetDataBorrowed::new::<BidiControl>();
const IGNORABLE: CodePointSetDataBorrowed =
    CodePointSetDataBorrowed::new::<DefaultIgnorableCodePoint>();

const MAX_VERSION_LEN_BYTES: u8 = 30;
const MAX_IRI_LEN_BYTES: u8 = 120;

/// Normalize IRI according to our spec.
/// A precondition for `iri` is that it must not have a fragment.
pub fn normalize_iri<T: Bos<str>>(iri: &Iri<T>) -> String {
    debug_assert!(!iri.has_fragment());
    // 1. Make IRI canonical.
    // 1. Decode punycode host/domain(s) if applicable according to URL standard.
    // 1. If IRI satisfies Sysand PURL requirements, set filename to `publisher-name`
    //    and go to end.
    // 1. Remove all percent-encoded octets that are not (parts of) valid UTF-8 characters.
    //    Note that these are not decoded during canonicalization, because they are not allowed
    //    in IRIs. See also [converting URIs to IRIs](https://datatracker.ietf.org/doc/html/rfc3987#section-3.2)
    // 1. Decode all percent-encoded octets. Note that normalization is preserved, since
    //    reserved characters are all ASCII, and at this point only syntactically reserved
    //    characters are still percent-encoded.
    // 1. Shorten the IRI by dropping non-informative components to format
    //    `host-path-query`. Non-existent parts will be omitted. `.kpar` extension
    //    is dropped if present at end of `path`.
    // 1. Strip all beginning/ending non-alphanumeric ASCII and spacing/control
    //    non-ASCII characters.
    //    Rationale: prevent problems with Windows, do not make files hidden on Unix. Also
    //    avoid problems with invisible characters.
    // 1. Replace invalid/undesirable ASCII punctuation/whitespace:

    //     - any amount of whitespace with any surrounding non-alphanumeric ASCII chars
    //       with a single `_`
    //     - `/` or `:` with `.`. Having a `.` requires considering Windows "special paths"
    //       like `CON`, `LPT` and such - cannot have names that start with these bare or if
    //       they are followed by a `.` followed by anything.
    //     - any amount of illegal ASCII characters or multiple punctuation symbols
    //       with a single `-`

    //     Note that since Unicode (non-ASCII) punctuation/whitespace has no special
    //     treatment in shells/OSs (at least common ones), it's left as-is

    // 1. Apply full Unicode case folding.
    //    Rationale: avoid issues with case sensitivity differences
    // 1. Normalize using Unicode NFC.
    //    Rationale: macOS is normalization-insensitive, normalizing avoids unexpected duplicate
    //    names
    // 1. Truncate to 120 bytes. If ending bytes now form an invalid UTF-8 char, remove them
    // 1. Strip all ending non-alphanumeric ASCII and spacing/control
    //    non-ASCII characters.

    let mut result = String::new();
    let canonical = canonicalize_iri(iri);

    // Special case PURL of the form: `pkg:sysand/<publisher>/<name>`
    let mut part_it = canonical.as_str().split('/');
    if let Some("pkg:sysand") = part_it.next()
        && let Some(publisher) = part_it.next()
        && is_valid_publisher(publisher)
        && let Some(name) = part_it.next()
        && is_valid_name(name)
        && part_it.next().is_none()
    {
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
                        if let Some(puny_decoded) = idna::punycode::decode_to_string(c) {
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
                    if let Some(puny_decoded) = idna::punycode::decode_to_string(c) {
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

    // Decode percent-encoded octets and strip out invalid
    extend_decode_strip_invalid(&mut result, canonical.path());
    // Strip `.kpar` suffix to reduce length
    if result.ends_with(".kpar") {
        result.truncate(result.len() - 5);
    }
    if let Some(query) = canonical.query() {
        result.push('-');
        extend_decode_strip_invalid(&mut result, query);
    }

    // 1. Strip all beginning/ending non-alphanumeric ASCII and spacing/control
    //    non-ASCII characters.
    //    Rationale: prevent problems with Windows, do not make files hidden on Unix. Also
    //    avoid problems with invisible characters.

    result = result
        .trim_matches(|c: char| {
            // !c.is_alphanumeric()
            (c.is_ascii() && !c.is_ascii_alphanumeric())
                || c.is_control()
                || c.is_whitespace()
                || IGNORABLE.contains(c)
        })
        .to_owned();

    // 1. Replace invalid/undesirable ASCII punctuation/whitespace:

    //     - any amount of whitespace with any surrounding non-alphanumeric ASCII chars
    //       with a single `_`
    //     - `/` or `:` with `.`. Having a `.` requires considering Windows "special paths"
    //       like `CON`, `LPT` and such - cannot have names that start with these bare or if
    //       they are followed by a `.` followed by anything.
    //     - any amount of illegal ASCII characters or multiple punctuation symbols
    //       with a single `-`

    //     Note that since Unicode (non-ASCII) punctuation/whitespace has no special
    //     treatment in shells/OSs (at least common ones), it's left as-is, except for
    //     bidi controls, as these if unterminated create a not-well-formed string
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

    // 1. Apply full Unicode case folding.
    //    Rationale: avoid issues with case sensitivity differences
    let casefolded = CASE_MAPPER.fold_string(&sanitized);

    // 1. Normalize using Unicode NFC.
    //    Rationale: macOS is normalization-insensitive, normalizing avoids unexpected duplicate
    //    names
    let nfc = NFC_NORMALIZER.normalize(&casefolded);
    // 1. Truncate to 120 bytes. If ending bytes now form an invalid UTF-8 char, remove them
    let truncation_boundary = nfc.floor_char_boundary(MAX_IRI_LEN_BYTES.into());
    let truncated = &nfc[..truncation_boundary];

    // 1. Strip all ending non-alphanumeric ASCII and spacing/control
    //    non-ASCII characters.

    truncated
        .trim_end_matches(|c: char| {
            // !c.is_alphanumeric()
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

/// Canonicalize IRI. The algorithm:
///
/// 1. Apply [standard IRI normalization from RFC3986/3987][iri-normalize]:
///
///   1. Decode any percent-encoded octets that correspond to an allowed character which is not reserved.
///   2. Uppercase the hexadecimal digits within all percent-encoded octets.
///   3. Lowercase all ASCII characters within the scheme and the host except the percent-encoded octets.
///   4. (not specified in 3986/7, but still useful) Turn any IPv6 literal address into its canonical form as per RFC 5952.
///   5. If the port is empty or equals the scheme's default, remove it along with the `:` delimiter.
///      This considers all IANA's permanently assigned ports.
///      We may want to customize this to only handle a fixed set of schemes to avoid
///      potential future changes to IANA's assigned port list
///   6. If IRI has a scheme and an absolute path, apply the `remove_dot_segments` algorithm
///      to the path, taking account of percent-encoded dot segments as described at `UriRef::resolve_against`.
///   7. If IRI has no authority and its path would start with `//`, prepend `/.` to the path.
///
/// 2. Ensure host/domain(s) are Punycode encoded, if applicable according to
///    [URL standard](https://url.spec.whatwg.org/#url-representation). Specifically,
///    run `domainToASCII` algorithm on them if they're not already in Punycode.
///    Note: since DNS _technically_ allows use of arbitrary bytes in domain names,
///    this _could_ create a false equivalence between actual name
///    and its Punycode-encoded counterpart, even though they might not be the
///    same actual host. The same applies to host/domain that begin with `xn--`,
///    but are not actually Punycode. This is deemed an acceptable risk, as such names
///    are expected to be disallowed by all domain registrars and are rejected by at
///    least some browsers.
///    Note that URL parsing (at least as implemented by Rust `url` crate) automatically
///    converts Unicode host/domain into Punycode.
/// 3. If scheme is HTTP(S) and path is empty, make path `/`
///
/// [iri-normalize]: https://docs.rs/fluent-uri/0.4.1/fluent_uri/struct.Iri.html#method.normalize
// TODO: use this in lock/env.toml generation
fn canonicalize_iri<T: Bos<str>>(iri: &Iri<T>) -> Iri<String> {
    let normal = iri.normalize();
    // TODO: this is mostly redundant, the only thing we need here is normalizing
    // Unicode domains to Punycode and turning empty path into slash.
    match Url::parse(normal.as_str()) {
        // Ok(url) => panic!("{url}"),
        Ok(url) => Iri::parse(String::from(url)).unwrap(),
        Err(e) => {
            log::debug!("failed to parse IRI `{normal}` as URL: {e}");
            normal
        }
    }
}

/// Normalize version according to our spec. We have to support arbitrary
/// version strings here, as `env install` supports arbitrary version strings
// TODO: update the spec to match implementation
pub fn normalize_version<V: AsRef<str>>(version: V) -> String {
    // Strip all beginning/ending non-alphanumeric ASCII and spacing/control
    // non-ASCII characters.
    let trimmed = version.as_ref().trim_matches(|c: char| {
        // !c.is_alphanumeric()
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
            // !c.is_alphanumeric()
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
    pub fn new<V: AsRef<str>, T: Bos<str>>(iri: &Iri<T>, version: V) -> Self {
        let mut normalized_iri = normalize_iri(iri);
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
#[path = "./utils_tests.rs"]
mod tests;
