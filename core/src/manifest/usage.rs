// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! One entry of `inline-usages`: a project this project uses.
//!
//! model-toml.md "Usages": "what a manifest declares is a *usage*
//! (`dependency` clashes with SysML)". An entry appears in a project's own
//! array, where it may instead be an opt-in to a usage the workspace
//! declares ([`MemberUsage`]), or in a workspace group's array, where it
//! is always declared ([`Usage`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use fluent_uri::Iri;
use serde::Serialize;
use typed_path::Utf8UnixPathBuf;

use crate::manifest::common::InheritMarker;
use crate::manifest::common::{
    InheritFrom, NormalizedIdentity, OMG_PUBLISHER, normalized_identity,
};
use crate::manifest::diagnostics::{
    Diagnostic, KeyLocation, Position, Report, UnknownKeys, missing_key,
};
use crate::manifest::reader::describe;
use crate::manifest::spans::Positions;
use crate::model::{IndexName, IndexPublisher, is_omg_stdlib_iri};

/// The keys a usage entry accepts, quoted as the
/// message lists them. One constant, so that the message and the fields
/// of [`UsageRaw`] cannot drift apart.
const ACCEPTED_KEYS: &str =
    "`publisher`, `name`, `version`, `dir`, `kpar-path`, `kpar-url`, `workspace`";

/// How a missing half of an identity is spelled, as the publisher
/// mismatch of a json target already spells it.
const MISSING: &str = "<none>";

/// One project a manifest uses, with the kind it is resolved by.
///
/// model-toml.md: "Which kind an entry declares is decided by its kind key
/// -- `dir`, `kpar-path`, `git`, `kpar-url` -- of which at most one may be
/// present; an index usage is the entry that carries none." The `git`
/// kind is "postponed to after v1: not declarable in v1" and so has no
/// variant here; `resource`, the KerML usage type json projects carry,
/// "cannot be declared in `model.toml`" either and only exists on the
/// KPAR side.
///
/// Every variant carries `publisher` and `name` as written: "that pair is
/// the identity a usage is resolved by", and it is not normalized here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Usage {
    /// `source path: dir, publisher, name`.
    SourcePath {
        publisher: IndexPublisher,
        name: IndexName,
        dir: Utf8UnixPathBuf,
    },
    /// `kpar path: kpar-path, publisher, name`.
    KparPath {
        publisher: IndexPublisher,
        name: IndexName,
        kpar_path: Utf8UnixPathBuf,
    },
    /// `index: publisher, name, version constraint` -- the entry that
    /// carries no kind key at all. "Index usages are implicit in v1: the
    /// entry names no index."
    Index {
        publisher: IndexPublisher,
        name: IndexName,
        version_constraint: semver::VersionReq,
    },
    /// `remote-kpar: kpar-url, publisher, name`; "version constraint not
    /// supported".
    ///
    /// The publisher is optional, and absent exactly for a
    /// standard-library usage: model-toml.md "Standard library" says
    /// `publisher` "is absent, which is allowed here and nowhere else".
    /// Such a usage's identity is `(omg, <name>)`.
    RemoteKpar {
        publisher: Option<IndexPublisher>,
        name: IndexName,
        url: Iri<String>,
    },
}

impl Usage {
    /// The publisher as *written*, which a standard-library usage does
    /// not write.
    ///
    /// Deliberately not `(omg, ...)`'s `omg`: that is the identity this
    /// usage resolves under, not a word its author typed, and a
    /// workspace group opt-in, which matches literally, or a label, which
    /// quotes the entry back, would be wrong to see it.
    /// [`Self::identity`] is where the fallback belongs.
    pub fn publisher(&self) -> Option<&IndexPublisher> {
        match self {
            Self::SourcePath { publisher, .. }
            | Self::KparPath { publisher, .. }
            | Self::Index { publisher, .. } => Some(publisher),
            Self::RemoteKpar { publisher, .. } => publisher.as_ref(),
        }
    }

    pub fn name(&self) -> &IndexName {
        match self {
            Self::SourcePath {
                name,
                publisher: _,
                dir: _,
            }
            | Self::KparPath {
                name,
                publisher: _,
                kpar_path: _,
            }
            | Self::Index {
                name,
                publisher: _,
                version_constraint: _,
            }
            | Self::RemoteKpar {
                name,
                publisher: _,
                url: _,
            } => name,
        }
    }

    /// model-toml.md "Normalization": "All projects are resolved by their
    /// *normalized* identity, regardless of how the usage is declared."
    /// model-toml.md "Standard library": a usage that declares no
    /// publisher "has the identity `(omg, <name>)`".
    pub fn identity(&self) -> NormalizedIdentity {
        normalized_identity(self.publisher().unwrap_or(&OMG_PUBLISHER), self.name())
    }

    /// Whether this is a standard-library usage: a `kpar-url` at an OMG
    /// standard-library IRI that declares no publisher.
    pub fn is_stdlib(&self) -> bool {
        matches!(
            self,
            Self::RemoteKpar {
                publisher: None,
                ..
            }
        )
    }

    /// The `dir`/`kpar-path` of a local usage, as written.
    pub fn local_path(&self) -> Option<&Utf8UnixPathBuf> {
        match self {
            Self::SourcePath {
                dir,
                publisher: _,
                name: _,
            } => Some(dir),
            Self::KparPath {
                kpar_path,
                publisher: _,
                name: _,
            } => Some(kpar_path),
            Self::Index {
                publisher: _,
                name: _,
                version_constraint: _,
            }
            | Self::RemoteKpar {
                publisher: _,
                name: _,
                url: _,
            } => None,
        }
    }
}

/// Which directory a usage's relative paths are joined onto.
///
/// workspaces.md "Common usages": a group entry's `dir`/`kpar-path` "is
/// relative to the directory of the `model.toml` that defines the
/// workspace, i.e. the workspace root, whichever member opts into it. A
/// member's own `[[inline-usages]]` stay relative to the member." The base
/// is therefore a property of where the usage came from, not of the usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageBase {
    /// The project's own directory.
    Project,
    /// The workspace root.
    WorkspaceRoot,
}

/// A usage together with the directory its paths are relative to. This is
/// what a resolved project's usages are, and what a resolver joins the
/// paths onto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatedUsage {
    pub usage: Usage,
    pub base: UsageBase,
}

impl LocatedUsage {
    pub fn declared(usage: Usage) -> Self {
        Self {
            usage,
            base: UsageBase::Project,
        }
    }

    pub fn inherited(usage: Usage) -> Self {
        Self {
            usage,
            base: UsageBase::WorkspaceRoot,
        }
    }

    /// The path a resolver opens for a local usage, given the project's
    /// and the workspace root's directories.
    ///
    /// Both of those are filesystem paths (hence `camino`) and the usage's
    /// own is a manifest path (`Utf8UnixPathBuf`); this join is the one
    /// boundary between the two.
    ///
    /// # Panics
    ///
    /// If an inherited usage is resolved without a workspace root, which
    /// cannot happen: a usage is only inherited by resolving it against
    /// the workspace it came from.
    pub fn resolve_local_path(
        &self,
        project_dir: &camino::Utf8Path,
        workspace_root: Option<&camino::Utf8Path>,
    ) -> Option<camino::Utf8PathBuf> {
        let relative = self.usage.local_path()?;
        let base = match self.base {
            UsageBase::Project => project_dir,
            UsageBase::WorkspaceRoot => {
                workspace_root.expect("an inherited usage comes from a workspace")
            }
        };
        Some(base.join(relative.as_str()))
    }
}

/// One `[[inline-usages]]` entry of a *project*.
///
/// workspaces.md "Common usages": "Members opt in per usage, not for the
/// array as a whole: an entry that gives only the identity plus a
/// `workspace` marker takes the matching group entry whole." A member may
/// declare its own usages alongside inherited ones, so an entry is either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemberUsage {
    Declared(Usage),
    Inherited {
        publisher: IndexPublisher,
        name: IndexName,
        from: InheritFrom,
        /// Where the `workspace` key that made this an opt-in is.
        ///
        /// Kept for the same reason
        /// [`crate::manifest::inherit::Inherited`] keeps one: the
        /// diagnostics an opt-in can produce are raised once the
        /// workspace is known, by which time the positions of this
        /// file are no longer at hand.
        at: Option<Position>,
    },
}

impl MemberUsage {
    pub fn identity(&self) -> NormalizedIdentity {
        match self {
            Self::Declared(usage) => usage.identity(),
            Self::Inherited {
                publisher,
                name,
                from: _,
                at: _,
            } => normalized_identity(publisher, name),
        }
    }
}

/// What a duplicate check needs of an entry, whichever array it came from.
pub trait UsageEntry {
    /// model-toml.md "Normalization": duplicates are the one comparison
    /// that normalizes before resolution.
    fn normalized_identity(&self) -> NormalizedIdentity;
    /// The entry as written, for the message.
    fn label(&self) -> String;
}

impl UsageEntry for Usage {
    fn normalized_identity(&self) -> NormalizedIdentity {
        self.identity()
    }

    fn label(&self) -> String {
        self.to_string()
    }
}

impl UsageEntry for MemberUsage {
    fn normalized_identity(&self) -> NormalizedIdentity {
        self.identity()
    }

    fn label(&self) -> String {
        match self {
            Self::Declared(usage) => usage.label(),
            Self::Inherited {
                publisher,
                name,
                from: _,
                at: _,
            } => format!("{publisher}/{name}"),
        }
    }
}

/// Where one entry sits in the document: which array of tables holds it,
/// named by its dotted path (`inline-usages`,
/// `workspace.group.legacy.inline-usages`), and its index in that array.
///
/// Carried per entry rather than derived from a position in a list,
/// because what reaches a duplicate check is the entries that *validated*
/// and one rejected entry would shift every later index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageSite<'a> {
    pub array: &'a str,
    pub index: usize,
}

impl UsageSite<'_> {
    fn segments(&self) -> Vec<String> {
        let mut segments: Vec<String> = self.array.split('.').map(ToOwned::to_owned).collect();
        segments.push(self.index.to_string());
        segments
    }

    /// Where the entry as a whole is: its `[[...]]` header.
    fn position(
        &self,
        positions: &Positions<'_>,
    ) -> Option<crate::manifest::diagnostics::Position> {
        let segments = self.segments();
        positions.near(&borrowed(&segments))
    }

    /// Where one of the entry's keys is, or the entry itself when the key
    /// is not there.
    fn key(
        &self,
        key: &str,
        positions: &Positions<'_>,
    ) -> Option<crate::manifest::diagnostics::Position> {
        let mut segments = self.segments();
        segments.push(key.to_owned());
        positions.near(&borrowed(&segments))
    }
}

fn borrowed(segments: &[String]) -> Vec<&str> {
    segments.iter().map(String::as_str).collect()
}

/// The `serde`-facing shape of one entry.
///
/// model-toml.md "Reading and validating": "every field of every manifest
/// ... is optional at parse time", and "whether its value has the right
/// shape" is Sysand's to check, not serde's -- so every field is a plain
/// `String` and every value is parsed in [`UsageRaw::validate_member`].
/// The alternative, validating inside `Deserialize`, makes one bad value
/// anywhere abort the read of the whole file.
///
/// `git`, `rev`, `tag` and `branch` are fields rather than unknown keys
/// because model-toml.md names the git kind and postpones it; a reader
/// that called them unknown would say they will never exist.
///
/// Read from a [`toml::Table`] rather than through `Deserialize`
/// ([`UsageRaw::from_table`]): a derive fails the whole file on the first
/// value of the wrong TOML type, which is the one thing "reports every
/// problem it finds in one report" does not allow. `Serialize` stays,
/// for writing a usage back out through `From<Usage>`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct UsageRaw {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(rename = "kpar-path", skip_serializing_if = "Option::is_none")]
    pub kpar_path: Option<String>,
    /// The field keeps the name the types use; what a file spells is
    /// `kpar-url` (model-toml.md "Usages").
    #[serde(rename = "kpar-url", skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// workspaces.md "Common usages": the per-entry opt-in marker, written
    /// directly among the entry's keys (`workspace = true`), not as the
    /// `<key>.workspace` table a metadata key uses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<InheritMarker>,
    /// workspaces.md "Unknown keys": inside a usage entry an unknown key
    /// is an error, so this set must be empty for the entry to be
    /// accepted.
    #[serde(flatten)]
    pub unknown: UnknownKeys,
    /// The keys that were written with a value of the wrong TOML type,
    /// each with the type that was found there.
    ///
    /// The value itself is gone, so what such an entry declares cannot be
    /// decided from what is left: an entry with one of these is refused
    /// without asking it for a kind, which would otherwise report a
    /// wrong-typed `dir` as an index usage that is missing its `version`.
    #[serde(skip)]
    pub wrong_typed: BTreeMap<String, &'static str>,
}

impl UsageRaw {
    /// Read one entry out of the values a `model.toml` parsed into.
    ///
    /// Nothing is judged here and nothing is reported: this takes what
    /// has the right shape, remembers what does not, and leaves every
    /// diagnostic to [`UsageRaw::validate_member`], which is the one
    /// place that decides what an entry means.
    pub fn from_table(table: &toml::Table) -> Self {
        let mut raw = Self::default();
        let mut strings: BTreeMap<&str, String> = BTreeMap::new();
        let mut unknown: BTreeSet<String> = BTreeSet::new();

        for (key, value) in table {
            match key.as_str() {
                "publisher" | "name" | "version" | "dir" | "kpar-path" | "kpar-url" | "git"
                | "rev" | "tag" | "branch" => match value.as_str() {
                    Some(text) => {
                        strings.insert(key.as_str(), text.to_owned());
                    }
                    None => {
                        raw.wrong_typed.insert(key.clone(), describe(value));
                    }
                },
                // The one key of an entry that is neither a string nor a
                // table: a marker is a bool or a group name.
                "workspace" => match value {
                    toml::Value::Boolean(flag) => {
                        raw.workspace = Some(InheritMarker::Flag(*flag));
                    }
                    toml::Value::String(group) => {
                        raw.workspace = Some(InheritMarker::Group(group.clone()));
                    }
                    _ => {
                        raw.wrong_typed.insert(key.clone(), describe(value));
                    }
                },
                _ => {
                    unknown.insert(key.clone());
                }
            }
        }

        raw.publisher = strings.remove("publisher");
        raw.name = strings.remove("name");
        raw.version = strings.remove("version");
        raw.dir = strings.remove("dir");
        raw.kpar_path = strings.remove("kpar-path");
        raw.url = strings.remove("kpar-url");
        raw.git = strings.remove("git");
        raw.rev = strings.remove("rev");
        raw.tag = strings.remove("tag");
        raw.branch = strings.remove("branch");
        raw.unknown = unknown.into_iter().collect();
        raw
    }

    /// The entry as the messages name it: `<publisher>/<name>` as
    /// written, with `<none>` for a half that is missing.
    pub fn label(&self) -> String {
        let name = self.name.as_deref().unwrap_or(MISSING);
        match self.publisher.as_deref() {
            Some(publisher) => format!("{publisher}/{name}"),
            // A standard-library usage declares no publisher and is
            // spelled by its name alone; an entry that *should* have one
            // is told so separately, and `<none>/lib` would only make
            // every other message about it harder to read.
            None => name.to_owned(),
        }
    }

    /// Validate one entry of a *workspace group's* array, which has
    /// nothing above it to inherit from.
    pub fn validate_declared(
        self,
        site: &UsageSite<'_>,
        positions: &Positions<'_>,
    ) -> Result<Usage, Report> {
        match self.validate(site, positions, false)? {
            MemberUsage::Declared(usage) => Ok(usage),
            // A marker is refused while validating, so an entry of a
            // group's array never comes back inherited.
            MemberUsage::Inherited {
                publisher: _,
                name: _,
                from: _,
                at: _,
            } => unreachable!("a group entry cannot carry a marker"),
        }
    }

    /// Validate one entry of a *project's* array, where a `workspace`
    /// marker is an opt-in.
    pub fn validate_member(
        self,
        site: &UsageSite<'_>,
        positions: &Positions<'_>,
    ) -> Result<MemberUsage, Report> {
        self.validate(site, positions, true)
    }

    fn validate(
        self,
        site: &UsageSite<'_>,
        positions: &Positions<'_>,
        marker_allowed: bool,
    ) -> Result<MemberUsage, Report> {
        let mut report = Report::new();
        let label = self.label();

        for key in self.unknown.iter() {
            report.push(
                Diagnostic::error(format!(
                    "unknown key `{key}` in usage `{label}`; a usage entry accepts only \
                     {ACCEPTED_KEYS}"
                ))
                .at_opt(site.key(key, positions)),
            );
        }

        let git_keys: Vec<&str> = [
            ("git", self.git.is_some()),
            ("rev", self.rev.is_some()),
            ("tag", self.tag.is_some()),
            ("branch", self.branch.is_some()),
        ]
        .into_iter()
        .filter_map(|(key, present)| present.then_some(key))
        .collect();
        if let Some(first) = git_keys.first() {
            let keys = git_keys
                .iter()
                .map(|key| format!("`{key}`"))
                .collect::<Vec<_>>()
                .join(", ");
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` declares {keys}; git usages are postponed to after v1 and \
                     cannot be declared in `model.toml`"
                ))
                .at_opt(site.key(first, positions)),
            );
        }

        for (key, found) in &self.wrong_typed {
            let expected = if key == "workspace" {
                "a boolean or a string"
            } else {
                "a string"
            };
            report.push(
                Diagnostic::error(format!(
                    "`{key}` in usage `{label}` must be {expected}, but it is {found}"
                ))
                .at_opt(site.key(key, positions)),
            );
        }

        let name = self.checked_name(site, positions, &mut report);

        // The kind is settled before the publisher, because it decides
        // whether an absent `publisher` is allowed: model-toml.md
        // "Standard library" exempts a `kpar-url` naming an OMG
        // standard-library IRI, and nothing else.
        let declared_kind =
            if self.wrong_typed.is_empty() && self.workspace.is_none() && git_keys.is_empty() {
                self.checked_kind(site, positions, &mut report, &label)
            } else {
                None
            };
        let exempt = matches!(
            &declared_kind,
            Some(Kind::RemoteKpar(url)) if is_omg_stdlib_iri(url.as_str())
        );
        let publisher = self.checked_publisher(site, positions, &mut report, name.as_ref(), exempt);

        // What an entry *is* decides which further checks apply. An entry
        // carrying a `workspace` key is an opt-in even when the marker
        // itself turns out to be invalid, and one declaring the postponed
        // git kind is not an index usage -- so neither is then asked for
        // the `version` an index usage needs.
        let member = if !self.wrong_typed.is_empty() {
            // The values are gone, so there is nothing left to decide a
            // shape from.
            None
        } else if self.workspace.is_some() {
            let from = self.checked_marker(site, positions, &mut report, marker_allowed, &label);
            from.and_then(|from| {
                self.check_opt_in_is_bare(site, positions, &mut report, &label, &from);
                publisher
                    .flatten()
                    .zip(name)
                    .map(|(publisher, name)| MemberUsage::Inherited {
                        publisher,
                        name,
                        from,
                        at: site.key("workspace", positions),
                    })
            })
        } else if git_keys.is_empty() {
            declared_kind
                .zip(publisher.zip(name))
                .and_then(|(kind, (publisher, name))| {
                    kind.into_usage(publisher, name).map(MemberUsage::Declared)
                })
        } else {
            None
        };

        match member {
            Some(member) if !report.has_errors() => Ok(member),
            _ => {
                report.sort_by_position();
                Err(report)
            }
        }
    }

    fn checked_name(
        &self,
        site: &UsageSite<'_>,
        positions: &Positions<'_>,
        report: &mut Report,
    ) -> Option<IndexName> {
        let Some(raw) = self.name.as_deref() else {
            if self.wrong_typed.contains_key("name") {
                return None;
            }
            report.push(
                missing_key(
                    "name",
                    &KeyLocation::ArrayTable(site.array.to_owned()),
                    None,
                )
                .at_opt(site.position(positions)),
            );
            return None;
        };
        match IndexName::parse(raw) {
            Ok(name) => Some(name),
            Err(error) => {
                report
                    .push(Diagnostic::error(error.to_string()).at_opt(site.key("name", positions)));
                None
            }
        }
    }

    /// The publisher an entry declares.
    ///
    /// `None` is an error already reported; `Some(None)` is an entry
    /// that is *allowed* to declare none -- `exempt`, which the caller
    /// sets only for a `kpar-url` at an OMG standard-library IRI.
    fn checked_publisher(
        &self,
        site: &UsageSite<'_>,
        positions: &Positions<'_>,
        report: &mut Report,
        name: Option<&IndexName>,
        exempt: bool,
    ) -> Option<Option<IndexPublisher>> {
        let Some(raw) = self.publisher.as_deref() else {
            if self.wrong_typed.contains_key("publisher") {
                return None;
            }
            if exempt {
                return Some(None);
            }
            // The message names the usage by its name, so with no
            // name there is nothing to say but that the key is missing.
            report.push(match name {
                Some(name) => Diagnostic::error(format!(
                    "usage `{name}` has no `publisher`; only a standard-library usage, whose \
                     `kpar-url` is an OMG standard-library IRI, may omit it"
                ))
                .at_opt(site.position(positions)),
                None => missing_key(
                    "publisher",
                    &KeyLocation::ArrayTable(site.array.to_owned()),
                    None,
                )
                .at_opt(site.position(positions)),
            });
            return None;
        };
        match IndexPublisher::parse(raw) {
            Ok(publisher) => Some(Some(publisher)),
            Err(error) => {
                report.push(
                    Diagnostic::error(error.to_string()).at_opt(site.key("publisher", positions)),
                );
                None
            }
        }
    }

    fn checked_marker(
        &self,
        site: &UsageSite<'_>,
        positions: &Positions<'_>,
        report: &mut Report,
        allowed: bool,
        label: &str,
    ) -> Option<InheritFrom> {
        let marker = self.workspace.clone()?;
        if !allowed {
            // A workspace group's own entries have nothing above them to
            // inherit from.
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` sets a `workspace` marker, which only a project's \
                     `inline-usages` may do"
                ))
                .at_opt(site.key("workspace", positions)),
            );
            return None;
        }
        match InheritFrom::try_from(marker) {
            Ok(from) => Some(from),
            Err(error) => {
                report.push(
                    Diagnostic::error(error.to_string()).at_opt(site.key("workspace", positions)),
                );
                None
            }
        }
    }

    /// workspaces.md "Common usages": "The group entry is used as-is; the
    /// member cannot change any of its fields", so an entry carrying a
    /// marker may give nothing but the identity.
    fn check_opt_in_is_bare(
        &self,
        site: &UsageSite<'_>,
        positions: &Positions<'_>,
        report: &mut Report,
        label: &str,
        from: &InheritFrom,
    ) {
        let extra = [
            ("version", self.version.is_some()),
            ("dir", self.dir.is_some()),
            ("kpar-path", self.kpar_path.is_some()),
            ("kpar-url", self.url.is_some()),
        ]
        .into_iter()
        .find_map(|(key, present)| present.then_some(key));
        if let Some(key) = extra {
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` inherits from {from}, so it may set only `publisher`, \
                     `name` and `workspace`"
                ))
                .at_opt(site.key(key, positions)),
            );
        }
    }

    fn checked_kind(
        &self,
        site: &UsageSite<'_>,
        positions: &Positions<'_>,
        report: &mut Report,
        label: &str,
    ) -> Option<Kind> {
        let kinds = [
            self.dir.is_some(),
            self.kpar_path.is_some(),
            self.url.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        if kinds > 1 {
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` carries more than one of `dir`, `kpar-path`, `kpar-url`"
                ))
                .at_opt(site.position(positions)),
            );
            return None;
        }

        // Only the index kind's field set carries a version requirement.
        // Appendix A2 row 55: "`kpar-url` takes no version constraint",
        // which is what a standard-library usage runs into.
        let reject_version = |report: &mut Report| {
            if self.version.is_some() {
                report.push(
                    Diagnostic::error(format!(
                        "usage `{label}` sets `version`, which a `dir`, `kpar-path` or \
                         `kpar-url` usage does not take"
                    ))
                    .at_opt(site.key("version", positions)),
                );
            }
        };

        let local_path = |report: &mut Report, key: &str, raw: &str| {
            if is_normal_local_path(raw) {
                return Some(Utf8UnixPathBuf::from(raw));
            }
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` sets `{key} = \"{raw}\"`, which is not in the form a \
                     local path takes:\n`/`-separated, with `..` only as leading components, \
                     no `.` component, no repeated or trailing slash and no backslash"
                ))
                .at_opt(site.key(key, positions)),
            );
            None
        };

        if let Some(dir) = self.dir.as_deref() {
            reject_version(report);
            return local_path(report, "dir", dir).map(Kind::SourcePath);
        }
        if let Some(kpar_path) = self.kpar_path.as_deref() {
            reject_version(report);
            return local_path(report, "kpar-path", kpar_path).map(Kind::KparPath);
        }
        if let Some(url) = self.url.as_deref() {
            reject_version(report);
            let Ok(parsed) = Iri::parse(url.to_owned()) else {
                report.push(
                    Diagnostic::error(format!(
                        "usage `{label}` sets `kpar-url = \"{url}\"`, which is not a valid IRI"
                    ))
                    .at_opt(site.key("kpar-url", positions)),
                );
                return None;
            };
            return Some(Kind::RemoteKpar(parsed));
        }

        let Some(version) = self.version.as_deref() else {
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` names no `dir`, `kpar-path` or `kpar-url`, so it is an index \
                     usage; an index usage requires a `version` constraint"
                ))
                .at_opt(site.position(positions)),
            );
            return None;
        };
        let Ok(constraint) = semver::VersionReq::parse(version) else {
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` sets `version = \"{version}\"`, which is not a version \
                     requirement"
                ))
                .at_opt(site.key("version", positions)),
            );
            return None;
        };
        Some(Kind::Index(constraint))
    }
}

/// The kind an entry declares, before its identity is attached.
enum Kind {
    SourcePath(Utf8UnixPathBuf),
    KparPath(Utf8UnixPathBuf),
    Index(semver::VersionReq),
    RemoteKpar(Iri<String>),
}

impl Kind {
    /// `None` only if a kind that requires a publisher reached here
    /// without one, which `checked_publisher` exempts no kind but
    /// `RemoteKpar` from -- so it is a bug, not an input.
    fn into_usage(self, publisher: Option<IndexPublisher>, name: IndexName) -> Option<Usage> {
        Some(match self {
            Self::RemoteKpar(url) => Usage::RemoteKpar {
                publisher,
                name,
                url,
            },
            Self::SourcePath(dir) => Usage::SourcePath {
                publisher: publisher?,
                name,
                dir,
            },
            Self::KparPath(kpar_path) => Usage::KparPath {
                publisher: publisher?,
                name,
                kpar_path,
            },
            Self::Index(version_constraint) => Usage::Index {
                publisher: publisher?,
                name,
                version_constraint,
            },
        })
    }
}

/// model-toml.md "Normalization": "two entries whose identities normalize
/// to the same value are the same project declared twice and are
/// rejected, whether or not the two entries agree on kind or constraint.
/// The arrays are counted separately, since a workspace config holds
/// several."
///
/// Every colliding pair is reported, not just the first, so that one edit
/// round fixes the file; each diagnostic points at the later of the two,
/// which is the entry the reader would delete.
pub fn check_duplicates<E: UsageEntry>(
    entries: &[(UsageSite<'_>, E)],
    positions: &Positions<'_>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (index, (site, entry)) in entries.iter().enumerate() {
        let identity = entry.normalized_identity();
        let Some((_, first)) = entries[..index]
            .iter()
            .find(|(_, earlier)| earlier.normalized_identity() == identity)
        else {
            continue;
        };
        diagnostics.push(
            Diagnostic::error(format!(
                "usages `{}` and `{}` name the same project; two entries whose publisher and \
                 name normalize to the same value are a duplicate",
                first.label(),
                entry.label()
            ))
            .at_opt(site.position(positions)),
        );
    }
    diagnostics
}

impl From<Usage> for UsageRaw {
    fn from(usage: Usage) -> Self {
        let mut raw = Self {
            // A standard-library usage writes no `publisher` key.
            publisher: usage.publisher().map(ToString::to_string),
            name: Some(usage.name().to_string()),
            ..Self::default()
        };
        match usage {
            Usage::SourcePath {
                dir,
                publisher: _,
                name: _,
            } => raw.dir = Some(dir.to_string()),
            Usage::KparPath {
                kpar_path,
                publisher: _,
                name: _,
            } => raw.kpar_path = Some(kpar_path.to_string()),
            Usage::Index {
                version_constraint,
                publisher: _,
                name: _,
            } => raw.version = Some(version_constraint.to_string()),
            Usage::RemoteKpar {
                url,
                publisher: _,
                name: _,
            } => raw.url = Some(url.to_string()),
        }
        raw
    }
}

impl From<MemberUsage> for UsageRaw {
    fn from(usage: MemberUsage) -> Self {
        match usage {
            MemberUsage::Declared(usage) => Self::from(usage),
            MemberUsage::Inherited {
                publisher,
                name,
                from,
                at: _,
            } => Self {
                publisher: Some(publisher.to_string()),
                name: Some(name.to_string()),
                workspace: Some(InheritMarker::from(from)),
                ..Self::default()
            },
        }
    }
}

impl Display for Usage {
    /// A standard-library usage writes no publisher, so it is spelled by
    /// its name alone.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.publisher() {
            Some(publisher) => write!(f, "{}/{}", publisher, self.name()),
            None => write!(f, "{}", self.name()),
        }
    }
}

/// model-toml.md "Usages": a `dir` or `kpar-path` is taken exactly as
/// written, and admitted only in the one form Sysand writes:
/// `/`-separated, with no backslash or NUL, no `.` component, `..` only
/// as the leading components of a relative path, and no empty component
/// -- so no repeated or trailing slash. Unlike `sources` and `members`,
/// such a path may leave the directory it is relative to, which is what
/// the leading `..` are for. An absolute path is admitted as before, and
/// held to the same rules past its root.
fn is_normal_local_path(raw: &str) -> bool {
    if raw.contains(['\\', '\0']) {
        return false;
    }
    let (absolute, rest) = match raw.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (typed_path::Utf8WindowsPath::new(raw).is_absolute(), raw),
    };
    let mut leading = !absolute;
    rest.split('/').all(|part| match part {
        "" | "." => false,
        ".." => leading,
        _ => {
            leading = false;
            true
        }
    })
}

/// Convert a relative `dir` or `kpar-path` into the form
/// [`is_normal_local_path`] admits, for a command that writes one from a
/// path it was given: `.` components and empty ones are dropped, and a
/// `..` cancels the component before it unless that is a `..` too. `None`
/// when nothing is left, which names the directory the path is relative
/// to -- a project using itself.
///
/// The caller has already refused an absolute path, a backslash and a
/// NUL; this only rearranges components.
pub fn normalize_local_path(raw: &str) -> Option<Utf8UnixPathBuf> {
    let mut parts: Vec<&str> = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| Utf8UnixPathBuf::from(parts.join("/")))
}

#[cfg(test)]
#[path = "./usage_tests.rs"]
mod tests;
