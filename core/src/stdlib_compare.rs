// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Comparing a built archive with the standard library OMG published.
//!
//! `model-toml.md` "Standard library" makes this the acceptance
//! criterion: what `sysand build` produces from the workspace fixture
//! must be the published release. A library function rather than a test
//! helper, so that it can be run outside the test suite too.
//!
//! Three rules, each one a difference between "the same project" and
//! "the same bytes":
//!
//! * `.project.json` equal as JSON **values**, so key order does not
//!   matter, and with `versionConstraint` compared as a parsed
//!   `VersionReq` -- the published constraints are bare (`1.0.0`) and a
//!   build writes caret (`^1.0.0`), which are the same requirement and
//!   different strings.
//! * `.meta.json` the same way, ignoring `created` (a build records its
//!   own) and allowing a `checksum` map where the published file has
//!   none -- three of the ten ship without one, and a build always
//!   writes one.
//! * every source file byte-identical under the same relative path.

#![cfg(feature = "filesystem")]

use std::collections::BTreeMap;

use camino::Utf8Path;
use serde_json::Value;

use crate::project::{ProjectRead as _, local_kpar::LocalKParProjectRaw, utils::wrapfs};

/// One way a built archive differs from the published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    /// A key whose value differs, named by its JSON pointer.
    Value {
        file: String,
        at: String,
        built: String,
        published: String,
    },
    /// A source file the archive does not carry, or carries with
    /// different bytes.
    Source { path: String, reason: String },
    /// The archive could not be read at all.
    Unreadable(String),
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Value {
                file,
                at,
                built,
                published,
            } => write!(
                f,
                "`{file}` at `{at}`: built `{built}`, published `{published}`"
            ),
            Self::Source { path, reason } => write!(f, "source `{path}`: {reason}"),
            Self::Unreadable(reason) => write!(f, "the archive could not be read: {reason}"),
        }
    }
}

/// Compare a built archive with the published metadata and sources.
///
/// `sources` maps a relative path to the published bytes; it is a
/// parameter because the release's files are the other half of the
/// claim and are not in this repository.
pub fn compare_to_published(
    built: &LocalKParProjectRaw,
    published_info: &str,
    published_meta: &str,
    sources: &BTreeMap<String, Vec<u8>>,
) -> Vec<Difference> {
    let mut differences = Vec::new();

    let (info, meta) = match built.get_project() {
        Ok((Some(info), Some(meta))) => (info, meta),
        Ok(_) => {
            differences.push(Difference::Unreadable(
                "the archive has no `.project.json`/`.meta.json` pair".to_owned(),
            ));
            return differences;
        }
        Err(error) => {
            differences.push(Difference::Unreadable(error.to_string()));
            return differences;
        }
    };

    compare_json(
        ".project.json",
        &to_value(&info),
        &parse(published_info),
        &[],
        &mut differences,
    );
    compare_json(
        ".meta.json",
        &to_value(&meta),
        &parse(published_meta),
        // A build records its own creation time, and always writes a
        // checksum map even where the published file has none.
        &["created", "checksum"],
        &mut differences,
    );

    for (path, published) in sources {
        match built.read_source(camino::Utf8Path::new(path).as_str()) {
            Ok(mut reader) => {
                let mut bytes = Vec::new();
                match std::io::Read::read_to_end(&mut reader, &mut bytes) {
                    Ok(_) if &bytes == published => {}
                    Ok(_) => differences.push(Difference::Source {
                        path: path.clone(),
                        reason: format!(
                            "{} bytes built, {} published",
                            bytes.len(),
                            published.len()
                        ),
                    }),
                    Err(error) => differences.push(Difference::Source {
                        path: path.clone(),
                        reason: error.to_string(),
                    }),
                }
            }
            Err(error) => differences.push(Difference::Source {
                path: path.clone(),
                reason: error.to_string(),
            }),
        }
    }

    differences
}

fn to_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// Structural comparison, with `versionConstraint` compared as a parsed
/// requirement wherever it appears.
fn compare_json(
    file: &str,
    built: &Value,
    published: &Value,
    ignored: &[&str],
    differences: &mut Vec<Difference>,
) {
    compare_at(file, "", built, published, ignored, differences);
}

fn compare_at(
    file: &str,
    at: &str,
    built: &Value,
    published: &Value,
    ignored: &[&str],
    differences: &mut Vec<Difference>,
) {
    match (built, published) {
        (Value::Object(b), Value::Object(p)) => {
            let mut keys: Vec<&String> = b.keys().chain(p.keys()).collect();
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                if at.is_empty() && ignored.contains(&key.as_str()) {
                    // `checksum` may be added, never changed: a
                    // published map that exists must still match.
                    if key == "checksum" && p.contains_key(key) {
                        // fall through to the comparison
                    } else {
                        continue;
                    }
                }
                let next = format!("{at}/{key}");
                match (b.get(key), p.get(key)) {
                    (Some(b), Some(p)) => {
                        compare_at(file, &next, b, p, ignored, differences);
                    }
                    (built, published) => differences.push(Difference::Value {
                        file: file.to_owned(),
                        at: next,
                        built: render(built),
                        published: render(published),
                    }),
                }
            }
        }
        (Value::Array(b), Value::Array(p)) if b.len() == p.len() => {
            for (index, (b, p)) in b.iter().zip(p).enumerate() {
                compare_at(file, &format!("{at}/{index}"), b, p, ignored, differences);
            }
        }
        // A bare version and a caret are the same requirement.
        (Value::String(b), Value::String(p)) if at.ends_with("/versionConstraint") => {
            let parsed = |raw: &str| semver::VersionReq::parse(raw).ok();
            if parsed(b) != parsed(p) || (parsed(b).is_none() && b != p) {
                differences.push(Difference::Value {
                    file: file.to_owned(),
                    at: at.to_owned(),
                    built: b.clone(),
                    published: p.clone(),
                });
            }
        }
        _ if built == published => {}
        _ => differences.push(Difference::Value {
            file: file.to_owned(),
            at: at.to_owned(),
            built: built.to_string(),
            published: published.to_string(),
        }),
    }
}

fn render(value: Option<&Value>) -> String {
    value.map_or_else(|| "<absent>".to_owned(), ToString::to_string)
}

/// The published sources for one library, read from a release checkout.
pub fn published_sources(
    dir: &Utf8Path,
    names: &[String],
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut sources = BTreeMap::new();
    for name in names {
        let at = dir.join(name);
        let bytes = wrapfs::read(&at).map_err(|e| format!("cannot read `{at}`: {e}"))?;
        sources.insert(name.clone(), bytes);
    }
    Ok(sources)
}

#[cfg(test)]
#[path = "./stdlib_compare_tests.rs"]
mod tests;
