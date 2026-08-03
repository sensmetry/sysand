// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! User-configurable index locations: either a plain base URL that relative
//! index paths are appended to (the historical behavior), or a URL template
//! containing a `{path}` placeholder that the relative index path is
//! substituted into, percent-encoded as a single path segment. Templates
//! make it possible to reach indexes served through file-access APIs whose
//! URL structure is not "base + path", such as the GitLab repository files
//! API (`.../repository/files/{path}/raw?ref=<branch>`).
//!
//! Two placeholder spellings are supported, differing only in how `/` is
//! treated when the relative index path (for example
//! `some-publisher/some-project/1.0.0/project.kpar`) is substituted:
//!
//! - `{path}` — every byte outside RFC 3986 *unreserved* is
//!   percent-encoded, including `/` as `%2F`. This is what the GitLab
//!   repository files API expects.
//! - `{path_raw}` — `/` stays literal; each path segment is
//!   percent-encoded individually. For hosts that take the file path as
//!   ordinary URL path segments but need a suffix after it (for example
//!   Gitea's `.../raw/<path>?ref=<branch>`).
//!
//! The syntax is a deliberately restricted subset of [RFC 6570] URI
//! Templates: `{path}` behaves like RFC 6570 simple string expansion of
//! a single value and `{path_raw}` like reserved expansion (`{+path}`),
//! but only these two fixed names are accepted so that typos fail at
//! parse time instead of producing wrong URLs. Fixed custom markers in a
//! package-index template follow crates.io's `config.json` `dl` field
//! (`{crate}`, `{version}`).
//!
//! [RFC 6570]: https://www.rfc-editor.org/rfc/rfc6570

use core::fmt;
use core::str::FromStr;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use thiserror::Error;

/// Placeholder that expands to the fully percent-encoded relative index
/// path (`/` becomes `%2F`).
const PATH_PLACEHOLDER: &str = "{path}";

/// Placeholder that expands to the relative index path with literal `/`
/// separators (each segment percent-encoded individually).
const PATH_RAW_PLACEHOLDER: &str = "{path_raw}";

/// Percent-encode everything outside RFC 3986 unreserved
/// (`A-Z a-z 0-9 - . _ ~`). Notably `/` becomes `%2F`, which is what
/// file-access APIs like GitLab's expect for a path used as one URL
/// segment.
const PATH_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Error, Debug)]
pub enum IndexLocationError {
    #[error("invalid index URL `{url}`")]
    InvalidUrl {
        url: Box<str>,
        source: url::ParseError,
    },
    #[error(
        "index URL `{url}` is relative;\n\
         index locations must be absolute HTTP(S) URLs"
    )]
    RelativeUrl { url: Box<str> },
    #[error(
        "index URL `{url}` contains `%7B`/`%7D` (percent-encoded braces);\n\
         did you mean a `{{path}}` template? Placeholders must be written\n\
         literally, e.g. `.../repository/files/{{path}}/raw?ref=main`"
    )]
    PreEncodedPlaceholder { url: Box<str> },
    #[error(
        "index URL template `{url}` contains unknown placeholder `{placeholder}`{hint};\n\
         supported placeholders: `{{path}}` (percent-encoded, `/` becomes `%2F`) and\n\
         `{{path_raw}}` (literal `/` separators)"
    )]
    UnknownPlaceholder {
        url: Box<str>,
        placeholder: Box<str>,
        hint: &'static str,
    },
    #[error(
        "index URL template `{url}` contains a stray `{{` or `}}`;\n\
         supported placeholders are `{{path}}` and `{{path_raw}}`"
    )]
    StrayBrace { url: Box<str> },
    #[error(
        "index URL template `{url}` must contain a `{{path}}` or `{{path_raw}}` placeholder\n\
         exactly once, found {count} occurrences"
    )]
    PlaceholderCount { url: Box<str>, count: usize },
    #[error(
        "index URL template `{url}` expands to an invalid URL;\n\
         note that a placeholder may only appear in the path or query\n\
         of an absolute HTTP(S) URL"
    )]
    InvalidTemplate {
        url: Box<str>,
        source: url::ParseError,
    },
    #[error(
        "index URL template `{url}` is relative;\n\
         templates must be absolute HTTP(S) URLs"
    )]
    RelativeTemplate { url: Box<str> },
    #[error(
        "index URL `{url}` uses scheme `{scheme}`;\n\
         only `http` and `https` are supported"
    )]
    UnsupportedScheme { url: Box<str>, scheme: Box<str> },
    #[error(
        "index URL `{url}` includes username or password;\n\
         URL userinfo is not allowed"
    )]
    Userinfo { url: Box<str> },
    #[error(
        "index URL `{url}` includes a `#` fragment;\n\
         fragments are never sent to the server and are not allowed in index locations"
    )]
    Fragment { url: Box<str> },
}

/// How a relative index path is encoded when substituted into a template
/// placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathEncoding {
    /// `{path}`: the whole path is one percent-encoded unit, `/` → `%2F`.
    Encoded,
    /// `{path_raw}`: `/` stays literal, each segment percent-encoded.
    Raw,
}

impl PathEncoding {
    fn placeholder(self) -> &'static str {
        match self {
            Self::Encoded => PATH_PLACEHOLDER,
            Self::Raw => PATH_RAW_PLACEHOLDER,
        }
    }

    /// The separator inserted between consecutive path segments: `%2F` for
    /// `{path}` (so `/` is encoded away) and a literal `/` for
    /// `{path_raw}`.
    fn separator(self) -> &'static str {
        match self {
            Self::Encoded => "%2F",
            Self::Raw => "/",
        }
    }
}

/// A URL template with exactly one `{path}` or `{path_raw}` placeholder,
/// validated at construction. Stored as the text on either side of the
/// placeholder rather than as a `url::Url` because `url::Url` percent-encodes
/// `{` or `}` on parse, which would corrupt the placeholder. The prefix is
/// normalized at parse to the text `url::Url` serializes it as (lowercase
/// scheme and host, punycode, default port dropped, explicit root `/`), so
/// different spellings of one template `Display` identically; the suffix
/// stays as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexUrlTemplate {
    prefix: String,
    suffix: String,
    encoding: PathEncoding,
}

impl IndexUrlTemplate {
    /// Validate `raw` as an index URL template. `raw` must contain exactly
    /// one `{path}` or `{path_raw}` placeholder, no other `{...}` tokens
    /// or stray braces, no fragment, and must expand to an absolute
    /// HTTP(S) URL without userinfo, with the placeholder in the path or
    /// query. The literal prefix is normalized to canonical URL text, so
    /// `Display` renders one canonical spelling per template.
    fn parse(raw: &str) -> Result<Self, IndexLocationError> {
        let boxed_url = || redact_userinfo(raw);

        let encoding = validate_placeholders(raw)?;

        // Fragments are never sent to the server; in a template one is
        // always a mistake (and would swallow the placeholder if it
        // followed `#`).
        if raw.contains('#') {
            return Err(IndexLocationError::Fragment { url: boxed_url() });
        }

        let placeholder = encoding.placeholder();
        let split = raw
            .find(placeholder)
            .expect("BUG: validate_placeholders guarantees exactly one placeholder");
        let mut template = Self {
            prefix: raw[..split].to_owned(),
            suffix: raw[split + placeholder.len()..].to_owned(),
            encoding,
        };

        // Expand a probe that encodes to a percent-escape: hosts and
        // ports cannot contain `%`, so a placeholder in the authority
        // component fails to parse rather than validating silently.
        let expanded = match url::Url::parse(&template.build(["pr obe", "probe"])) {
            Ok(expanded) => expanded,
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                return Err(IndexLocationError::RelativeTemplate { url: boxed_url() });
            }
            Err(source) => {
                return Err(IndexLocationError::InvalidTemplate {
                    url: boxed_url(),
                    source,
                });
            }
        };
        validate_url_shape(raw, &expanded)?;

        // Rewrite the prefix into the text `url::Url` serializes it as
        // (lowercase scheme and host, punycode, default port dropped,
        // explicit root `/`), so `Display` output is canonical. The probe
        // keeps the prefix's end mid-segment, where it sits in every
        // expansion: parsed bare, a prefix ending in `.`, `..`, or a
        // space hits end-of-URL rules (dot-segment resolution, trailing
        // whitespace trimming) that never apply to the template. The
        // probe goes on the prefix alone so suffix text cannot move the
        // cut, and URL serialization keeps the probe's unreserved
        // characters and `%20` escape verbatim.
        let probe = "pr%20obe";
        let normalized = url::Url::parse(&format!("{}{probe}", template.prefix))
            .expect("BUG: the prefix of a validated template followed by the probe parses");
        template.prefix = normalized.into();
        debug_assert!(
            template.prefix.ends_with(probe),
            "BUG: URL serialization preserves the probe text"
        );
        template
            .prefix
            .truncate(template.prefix.len() - probe.len());

        Ok(template)
    }

    /// Build the expanded URL text from `segments`, each percent-encoded and
    /// joined by the placeholder's separator.
    fn build<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> String {
        let mut out = self.prefix.clone();
        for (i, segment) in segments.into_iter().enumerate() {
            if i > 0 {
                out.push_str(self.encoding.separator());
            }
            out.extend(utf8_percent_encode(segment, PATH_ENCODE_SET));
        }
        out.push_str(&self.suffix);
        out
    }

    /// The literal template text before the placeholder, in normalized
    /// form. Support for credential glob derivation (`commands::auth`),
    /// not general API: the prefix is normalized URL text but not a
    /// complete URL on its own (it can end mid-segment or mid-query).
    /// Gated like its only consumer so feature subsets stay dead-code-free.
    #[cfg(all(feature = "filesystem", feature = "networking"))]
    pub(crate) fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Substitute the relative index path `segments` (each percent-encoded
    /// according to the placeholder spelling) into the template. Infallible:
    /// the template was validated at construction and the segments encode to
    /// URL-safe bytes, so the result is always a valid URL.
    pub fn expand<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> url::Url {
        url::Url::parse(&self.build(segments))
            .expect("BUG: a validated index URL template expands to a valid URL")
    }
}

impl fmt::Display for IndexUrlTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}{}",
            self.prefix,
            self.encoding.placeholder(),
            self.suffix
        )
    }
}

/// Where an index lives: a plain base URL (relative index paths are
/// RFC 3986-joined onto it, the historical behavior) or a URL template
/// (relative index paths are percent-encoded into its `{path}`
/// placeholder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexLocation {
    /// A plain base URL. Constructed through [`Self::parse`], its path is
    /// normalized to end with `/` so that relative index paths append
    /// rather than replace the last segment; direct constructions must
    /// uphold the same invariant.
    Root(url::Url),
    Template(IndexUrlTemplate),
}

impl IndexLocation {
    /// Parse a user-supplied index location string. Strings containing `{`
    /// or `}` are treated as URL templates; anything else is parsed as a
    /// plain base URL. Braces are outside the URI character set
    /// ([RFC 3986 §2]), so no conformant URL is misclassified. (The `url`
    /// crate previously tolerated literal braces by percent-encoding them
    /// in paths and keeping them in queries; such non-conformant index
    /// URLs are now template-syntax errors.)
    ///
    /// Both variants enforce the same validity invariants at construction:
    /// an absolute HTTP(S) URL without userinfo or fragment. Later
    /// resolution is thus infallible.
    ///
    /// [RFC 3986 §2]: https://www.rfc-editor.org/rfc/rfc3986#section-2
    pub fn parse(s: &str) -> Result<Self, IndexLocationError> {
        if is_template_syntax(s) {
            return Ok(Self::Template(IndexUrlTemplate::parse(s)?));
        }
        // A pre-encoded `%7Bpath%7D` would silently behave as a plain
        // append-mode URL and 404 on every fetch; catch the paste
        // accident early instead. Only the exact placeholder spellings
        // are rejected, so URLs that legitimately contain encoded braces
        // elsewhere keep working. Gated on `%` so the common case does
        // not allocate.
        if s.contains('%') {
            let lower = s.to_ascii_lowercase();
            if lower.contains("%7bpath%7d") || lower.contains("%7bpath_raw%7d") {
                return Err(IndexLocationError::PreEncodedPlaceholder {
                    url: redact_userinfo(s),
                });
            }
        }
        let url = match url::Url::parse(s) {
            Ok(url) => url,
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                return Err(IndexLocationError::RelativeUrl {
                    url: redact_userinfo(s),
                });
            }
            Err(source) => {
                return Err(IndexLocationError::InvalidUrl {
                    url: redact_userinfo(s),
                    source,
                });
            }
        };
        // Enforce the same shape a template's expansion must satisfy. An
        // HTTP(S) URL can always be a base, so the trailing-slash
        // normalization (which `resolve` relies on) cannot fail.
        validate_url_shape(s, &url)?;
        Ok(Self::Root(with_trailing_slash(url)))
    }

    /// Construct the URL for the index file whose relative path is
    /// `segments` (for example `["index.json"]` or `["<publisher>",
    /// "<name>", "versions.json"]`). The segments are trusted: callers pass
    /// components validated upstream (charset-checked publisher/name,
    /// `_iri`/`<sha256hex>`, parsed semver versions, fixed file names — see
    /// `design/index-protocol.md` §5), so they cannot smuggle `..` or URL
    /// syntax into the result. Infallible, since both variants were
    /// shape-validated at construction.
    pub fn resolve<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> url::Url {
        match self {
            Self::Root(root) => {
                // Append each segment to a clone of the base rather than
                // building a `/`-joined string and reparsing it in
                // `join()`. The root ends with `/` (its invariant), so drop
                // that trailing empty segment first to avoid a doubled slash.
                let mut url = root.clone();
                url.path_segments_mut()
                    .expect("BUG: an HTTP(S) base URL can have path segments")
                    .pop_if_empty()
                    .extend(segments);
                url
            }
            Self::Template(template) => template.expand(segments),
        }
    }

    /// The plain base URL, if this location is not a template.
    pub fn as_root(&self) -> Option<&url::Url> {
        match self {
            Self::Root(url) => Some(url),
            Self::Template(_) => None,
        }
    }
}

impl fmt::Display for IndexLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root(url) => f.write_str(url.as_str()),
            Self::Template(template) => write!(f, "{template}"),
        }
    }
}

impl FromStr for IndexLocation {
    type Err = IndexLocationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Check that every `{...}` token in `raw` is `{path}` or `{path_raw}`,
/// that braces are balanced and non-nested, and that exactly one
/// placeholder occurs. Returns the encoding the placeholder selects.
fn validate_placeholders(raw: &str) -> Result<PathEncoding, IndexLocationError> {
    let boxed_url = || redact_userinfo(raw);
    let mut found: Vec<PathEncoding> = Vec::new();
    let mut rest = raw;
    while let Some(open) = rest.find(['{', '}']) {
        if rest.as_bytes()[open] == b'}' {
            return Err(IndexLocationError::StrayBrace { url: boxed_url() });
        }
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find(['{', '}']) else {
            return Err(IndexLocationError::StrayBrace { url: boxed_url() });
        };
        if after_open.as_bytes()[close] == b'{' {
            return Err(IndexLocationError::StrayBrace { url: boxed_url() });
        }
        let name = &after_open[..close];
        match name {
            "path" => found.push(PathEncoding::Encoded),
            "path_raw" => found.push(PathEncoding::Raw),
            _ => {
                let hint = if name.eq_ignore_ascii_case("path") {
                    " (did you mean `{path}`?)"
                } else if name.eq_ignore_ascii_case("path_raw") {
                    " (did you mean `{path_raw}`?)"
                } else {
                    ""
                };
                return Err(IndexLocationError::UnknownPlaceholder {
                    url: boxed_url(),
                    placeholder: format!("{{{name}}}").into(),
                    hint,
                });
            }
        }
        rest = &after_open[close + 1..];
    }
    match found.as_slice() {
        [encoding] => Ok(*encoding),
        _ => Err(IndexLocationError::PlaceholderCount {
            url: boxed_url(),
            count: found.len(),
        }),
    }
}

/// Enforce the shared index-location invariant on `url`: an absolute
/// HTTP(S) URL without userinfo or fragment. `reported` is the string
/// named in any error (the template text for a template, the URL itself
/// for a root).
fn validate_url_shape(reported: &str, url: &url::Url) -> Result<(), IndexLocationError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(IndexLocationError::UnsupportedScheme {
            url: redact_userinfo(reported),
            scheme: url.scheme().into(),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(IndexLocationError::Userinfo {
            url: redact_userinfo(reported),
        });
    }
    // Fragments are never sent to the server; on an index location one
    // is always a mistake. Templates reject `#` before parsing (it would
    // swallow the placeholder), so this arm fires for roots.
    if url.fragment().is_some() {
        return Err(IndexLocationError::Fragment {
            url: redact_userinfo(reported),
        });
    }
    Ok(())
}

/// Whether `s` uses index URL template syntax (contains a brace).
fn is_template_syntax(s: &str) -> bool {
    s.contains(['{', '}'])
}

/// Render a possibly malformed URL for an error message with any userinfo
/// replaced by `<redacted>`, so an embedded password never reaches stderr
/// or CI logs. String-based on purpose: it must work for inputs
/// `url::Url::parse` rejected.
fn redact_userinfo(raw: &str) -> Box<str> {
    // The authority runs from after any scheme separator to the first
    // `/`, `?`, or `#`; userinfo is everything up to the last `@` in it.
    let authority_start = raw.find("://").map_or(0, |idx| idx + 3);
    let authority_end = raw[authority_start..]
        .find(['/', '?', '#'])
        .map_or(raw.len(), |idx| authority_start + idx);
    match raw[authority_start..authority_end].rfind('@') {
        Some(at) => format!(
            "{}<redacted>@{}",
            &raw[..authority_start],
            &raw[authority_start + at + 1..]
        )
        .into(),
        None => raw.into(),
    }
}

/// Return `url` with a guaranteed trailing slash on its path so that
/// `Url::join` treats it as a directory. Operates via `path_segments_mut`
/// rather than touching the serialized path string, so percent-encoded
/// segments survive the round-trip unchanged.
///
/// Callers must pass an HTTP(S) URL. Such URLs can be a base, so the
/// `path_segments_mut` call should not fail after endpoint-shape
/// validation.
pub(crate) fn with_trailing_slash(mut url: url::Url) -> url::Url {
    {
        let mut segments = url
            .path_segments_mut()
            .expect("caller passes a URL that can be a base");
        segments.pop_if_empty();
        segments.push("");
    }
    url
}

#[cfg(test)]
#[path = "./index_location_tests.rs"]
mod tests;
