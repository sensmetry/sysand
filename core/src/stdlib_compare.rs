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

/// Where each standard library's sources sit in a release checkout.
///
/// A second table beside the embedded assets, because those are keyed by
/// KPAR stem and the release lays its libraries out by family and name.
/// Here rather than in a test because more than one test needs it. The
/// six domain libraries are spelled `<X>` in the release, not
/// `<X> Domain Library` after the KPAR stem.
pub fn release_paths() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        (
            "semantic-library",
            "sysml.library/Kernel Libraries/Kernel Semantic Library",
        ),
        (
            "data-type-library",
            "sysml.library/Kernel Libraries/Kernel Data Type Library",
        ),
        (
            "function-library",
            "sysml.library/Kernel Libraries/Kernel Function Library",
        ),
        ("systems-library", "sysml.library/Systems Library"),
        (
            "analysis-library",
            "sysml.library/Domain Libraries/Analysis",
        ),
        (
            "cause-and-effect-library",
            "sysml.library/Domain Libraries/Cause and Effect",
        ),
        (
            "geometry-library",
            "sysml.library/Domain Libraries/Geometry",
        ),
        (
            "metadata-library",
            "sysml.library/Domain Libraries/Metadata",
        ),
        (
            "quantities-and-units-library",
            "sysml.library/Domain Libraries/Quantities and Units",
        ),
        (
            "requirement-derivation-library",
            "sysml.library/Domain Libraries/Requirement Derivation",
        ),
    ])
}

/// Copy the standard-library workspace `fixture` into `into`, with each
/// member's sources taken from the release checkout at `sources_root`.
///
/// Copied rather than referenced: a `sources` entry is a relative path
/// that cannot escape its project root, so the files have to be *in* the
/// member directory for a build to find them.
///
/// The file list is each member's own declared `sources`, so this stages
/// exactly what the fixture says it publishes -- and a disagreement
/// between that list and the release shows up here rather than as a
/// comparison difference later.
pub fn stage_release_workspace(
    fixture: &Utf8Path,
    sources_root: &Utf8Path,
    into: &Utf8Path,
) -> Result<(), String> {
    let paths = release_paths();

    // Every library's directory first, and all of them before any copy.
    // A table that is wrong about the release layout should say so once,
    // naming each entry, rather than as "one file is missing".
    let absent: Vec<String> = paths
        .iter()
        .filter(|(_, relative)| !sources_root.join(relative).is_dir())
        .map(|(asset, relative)| format!("`{asset}` expects `{relative}`"))
        .collect();
    if !absent.is_empty() {
        return Err(format!(
            "{} of {} libraries are not where `release_paths` says, under \
             `{sources_root}` -- is it a checkout of the release?\n{}",
            absent.len(),
            paths.len(),
            absent.join("\n")
        ));
    }

    wrapfs::copy(fixture.join("model.toml"), into.join("model.toml"))
        .map_err(|e| format!("copying the root manifest: {e}"))?;

    for (asset, relative) in &paths {
        let member = into.join(asset);
        wrapfs::create_dir_all(&member).map_err(|e| e.to_string())?;
        let manifest = fixture.join(asset).join("model.toml");
        wrapfs::copy(&manifest, member.join("model.toml"))
            .map_err(|e| format!("copying `{asset}`'s manifest: {e}"))?;

        let from = sources_root.join(relative);
        for name in declared_sources(&manifest)? {
            let at = from.join(&name);
            wrapfs::copy(&at, member.join(&name)).map_err(|e| format!("copying `{at}`: {e}"))?;
        }
    }
    Ok(())
}

/// The `sources` a member's `model.toml` declares.
///
/// Read as text rather than through the manifest reader: a member of the
/// standard-library workspace inherits its metamodel from the table, so
/// reading it as a standalone project is an error by construction.
fn declared_sources(manifest: &Utf8Path) -> Result<Vec<String>, String> {
    let text =
        wrapfs::read_to_string(manifest).map_err(|e| format!("reading `{manifest}`: {e}"))?;
    let table: toml::Table = text
        .parse()
        .map_err(|e| format!("parsing `{manifest}`: {e}"))?;
    Ok(table
        .get("project")
        .and_then(|project| project.get("sources"))
        .and_then(toml::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

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
