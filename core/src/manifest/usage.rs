// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! One entry of `inline-usages`: a project this project uses.
//!
//! What a manifest declares is a *usage* (`dependency` clashes with
//! SysML). An entry appears in a project's own array, where it may
//! instead be an opt-in to a usage the workspace declares
//! ([`MemberUsage`]), or in a workspace group's array, where it is always
//! declared ([`Usage`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use fluent_uri::Iri;
use serde::Serialize;
use toml::de::{DeTable, DeValue};
use typed_path::Utf8UnixPathBuf;

use crate::manifest::common::InheritMarker;
use crate::manifest::common::{
    InheritFrom, NormalizedIdentity, OMG_PUBLISHER, normalized_identity,
};
use crate::manifest::diagnostics::{
    Diagnostic, Diagnostics, KeyLocation, Position, UnknownKeys, missing_key,
};
use crate::manifest::reader::{TableReader, describe};
use crate::model::{IndexName, IndexPublisher, is_omg_stdlib_iri};

/// The keys a usage entry accepts, quoted as the message lists them.
const ACCEPTED_KEYS: &str =
    "`publisher`, `name`, `version`, `dir`, `kpar-path`, `kpar-url`, `workspace`";

/// How a missing half of an identity is spelled in messages.
const MISSING: &str = "<none>";

/// One project a manifest uses, with the kind it is resolved by.
///
/// The kind an entry declares is decided by its kind key (`dir`,
/// `kpar-path`, `kpar-url`), of which at most one may be present; an
/// index usage is the entry that carries none. The `git` kind is not
/// declarable and has no variant here, and neither has `resource`.
///
/// Every variant carries `publisher` and `name` as written, not
/// normalized.
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
    /// `index: publisher, name, version constraint`: the entry that
    /// carries no kind key at all. It names no index.
    Index {
        publisher: IndexPublisher,
        name: IndexName,
        version_constraint: semver::VersionReq,
    },
    /// `remote-kpar: kpar-url, publisher, name`; no version constraint.
    ///
    /// The publisher is absent exactly for a standard-library usage,
    /// whose identity is `(omg, <name>)`.
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
    /// The fallback to `omg` is applied by [`Self::identity`].
    pub fn publisher(&self) -> Option<&IndexPublisher> {
        match self {
            Self::SourcePath {
                publisher,
                name: _,
                dir: _,
            }
            | Self::KparPath {
                publisher,
                name: _,
                kpar_path: _,
            }
            | Self::Index {
                publisher,
                name: _,
                version_constraint: _,
            } => Some(publisher),
            Self::RemoteKpar {
                publisher,
                name: _,
                url: _,
            } => publisher.as_ref(),
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

    /// The normalized identity; a usage that declares no publisher has the
    /// identity `(omg, <name>)`.
    pub fn identity(&self) -> NormalizedIdentity {
        normalized_identity(self.publisher().unwrap_or(&OMG_PUBLISHER), self.name())
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
/// A group entry's `dir`/`kpar-path` is relative to the workspace root,
/// whichever member opts into it. A member's own `[[inline-usages]]` stay
/// relative to the member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageBase {
    /// The project's own directory.
    Project,
    /// The workspace root.
    WorkspaceRoot,
}

/// A usage together with the directory its paths are relative to.
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
}

/// One `[[inline-usages]]` entry of a *project*.
///
/// Members opt in per usage: an entry that
/// gives only the identity plus a `workspace` marker takes the matching
/// group entry whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemberUsage {
    Declared(Usage),
    Inherited {
        /// Absent exactly when the entry opts into a *standard-library*
        /// group entry, which carries no publisher; the opt-in likewise
        /// gives only `name` and the marker.
        publisher: Option<IndexPublisher>,
        name: IndexName,
        from: InheritFrom,
        /// Where the `workspace` key that made this an opt-in is, kept for
        /// diagnostics raised once the workspace is known.
        at: Option<Position>,
    },
}

impl MemberUsage {
    pub fn identity(&self) -> NormalizedIdentity {
        match self {
            Self::Declared(usage) => usage.identity(),
            // An opt-in that omits its publisher opts into the standard library.
            Self::Inherited {
                publisher,
                name,
                from: _,
                at: _,
            } => normalized_identity(publisher.as_ref().unwrap_or(&OMG_PUBLISHER), name),
        }
    }
}

/// What a duplicate check needs of an entry, whichever array it came from.
pub trait UsageEntry {
    /// Duplicates are compared after normalization.
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
            // Spelled as written.
            Self::Inherited {
                publisher: Some(publisher),
                name,
                from: _,
                at: _,
            } => format!("{publisher}/{name}"),
            Self::Inherited {
                publisher: None,
                name,
                from: _,
                at: _,
            } => name.to_string(),
        }
    }
}

/// Where one entry sits in the document: which array of tables holds it,
/// named by its dotted path (`inline-usages`,
/// `workspace.group.legacy.inline-usages`), its index in that array, and
/// the entry itself, which positions what is said about it.
#[derive(Clone, Copy, Debug)]
pub struct UsageSite<'a> {
    pub array: &'a str,
    pub index: usize,
    pub entry: &'a TableReader<'a>,
}

impl UsageSite<'_> {
    /// Where the entry as a whole is: its `[[...]]` header.
    fn position(&self) -> Option<Position> {
        self.entry.here()
    }

    /// Where one of the entry's keys is, or the entry itself when the key
    /// is not there.
    fn key(&self, key: &str) -> Option<Position> {
        self.entry.at(key)
    }
}

/// The `serde`-facing shape of one entry.
///
/// Every value is a plain `String`, parsed during validation.
///
/// `git`, `rev`, `tag` and `branch` are recognized fields because the git
/// kind is reserved.
///
/// Read from a [`DeTable`] by [`UsageRaw::from_table`]. `Serialize`
/// writes a usage back out through `From<Usage>`.
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
    /// Spelled `kpar-url` in a file.
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
    /// The per-entry opt-in marker, written directly among the entry's
    /// keys (`workspace = true`), unlike the `<key>.workspace` table a
    /// metadata key uses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<InheritMarker>,
    /// An unknown key in a usage entry is an error, so this must be empty
    /// for the entry to be accepted.
    #[serde(flatten)]
    pub unknown: UnknownKeys,
    /// The keys that were written with a value of the wrong TOML type,
    /// each with the type that was found there.
    ///
    /// The value itself is gone, so an entry with one of these is refused
    /// without asking it for a kind, which would report a wrong-typed `dir`
    /// as an index usage that is missing its `version`.
    #[serde(skip)]
    pub wrong_typed: BTreeMap<String, &'static str>,
}

impl UsageRaw {
    /// Read one entry out of the values a `model.toml` parsed into.
    ///
    /// Nothing is judged or reported here: what has the right shape is
    /// taken, and what does not is remembered for validation.
    pub fn from_table(table: &DeTable<'_>) -> Self {
        let mut raw = Self::default();
        let mut unknown: BTreeSet<String> = BTreeSet::new();

        for (key, value) in table {
            let key: &str = key.get_ref();
            let value = value.get_ref();
            let field = match key {
                "publisher" => &mut raw.publisher,
                "name" => &mut raw.name,
                "version" => &mut raw.version,
                "dir" => &mut raw.dir,
                "kpar-path" => &mut raw.kpar_path,
                "kpar-url" => &mut raw.url,
                "git" => &mut raw.git,
                "rev" => &mut raw.rev,
                "tag" => &mut raw.tag,
                "branch" => &mut raw.branch,
                // The one key of an entry that is neither a string nor a
                // table: a marker is a bool or a group name.
                "workspace" => {
                    match value {
                        DeValue::Boolean(flag) => {
                            raw.workspace = Some(InheritMarker::Flag(*flag));
                        }
                        DeValue::String(group) => {
                            raw.workspace = Some(InheritMarker::Group(group.clone().into_owned()));
                        }
                        _ => {
                            raw.wrong_typed
                                .insert(key.to_owned(), describe(value.type_str()));
                        }
                    }
                    continue;
                }
                _ => {
                    unknown.insert(key.to_owned());
                    continue;
                }
            };
            match value {
                DeValue::String(text) => *field = Some(text.clone().into_owned()),
                _ => {
                    raw.wrong_typed
                        .insert(key.to_owned(), describe(value.type_str()));
                }
            }
        }

        raw.unknown = unknown.into_iter().collect();
        raw
    }

    /// The entry as the messages name it: `<publisher>/<name>` as
    /// written, with `<none>` for a half that is missing.
    pub fn label(&self) -> String {
        let name = self.name.as_deref().unwrap_or(MISSING);
        match self.publisher.as_deref() {
            Some(publisher) => format!("{publisher}/{name}"),
            // A standard-library usage is spelled by its name alone.
            None => name.to_owned(),
        }
    }

    /// Validate one entry of a *workspace group's* array, which has
    /// nothing above it to inherit from.
    pub fn validate_declared(self, site: &UsageSite<'_>) -> Result<Usage, Diagnostics> {
        match self.validate(site, false)? {
            MemberUsage::Declared(usage) => Ok(usage),
            // Markers are refused by `validate` here, so this is unreachable.
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
    pub fn validate_member(self, site: &UsageSite<'_>) -> Result<MemberUsage, Diagnostics> {
        self.validate(site, true)
    }

    fn validate(
        self,
        site: &UsageSite<'_>,
        marker_allowed: bool,
    ) -> Result<MemberUsage, Diagnostics> {
        let mut report = Diagnostics::new();
        let label = self.label();

        for key in self.unknown.iter() {
            report.push(
                Diagnostic::error(format!(
                    "unknown key `{key}` in usage `{label}`; a usage entry accepts only \
                     {ACCEPTED_KEYS}"
                ))
                .at_opt(site.key(key)),
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
                .at_opt(site.key(first)),
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
                .at_opt(site.key(key)),
            );
        }

        let name = self.checked_name(site, &mut report);

        // The kind is settled before the publisher, because it decides
        // whether an absent `publisher` is allowed.
        let declared_kind =
            if self.wrong_typed.is_empty() && self.workspace.is_none() && git_keys.is_empty() {
                self.checked_kind(site, &mut report, &label)
            } else {
                None
            };
        // Two exemptions: a `kpar-url` at an OMG standard-library IRI, and
        // an opt-in, which takes its publisher (or its absence) from the
        // group entry it names. Keyed on the raw `workspace` key, as the
        // marker is checked later.
        let exempt = self.workspace.is_some()
            || matches!(
                &declared_kind,
                Some(Kind::RemoteKpar(url)) if is_omg_stdlib_iri(url.as_str())
            );
        let publisher = self.checked_publisher(site, &mut report, name.as_ref(), exempt);

        // An entry carrying a `workspace` key is an opt-in even when the
        // marker is invalid, and one declaring the postponed git kind is
        // not an index usage, so neither is asked for the `version` an
        // index usage needs.
        let member = if !self.wrong_typed.is_empty() {
            // The values are gone, so no shape can be decided.
            None
        } else if self.workspace.is_some() {
            let from = self.checked_marker(site, &mut report, marker_allowed, &label);
            from.and_then(|from| {
                self.check_opt_in_is_bare(site, &mut report, &label, &from);
                publisher
                    .zip(name)
                    .map(|(publisher, name)| MemberUsage::Inherited {
                        publisher,
                        name,
                        from,
                        at: site.key("workspace"),
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

    fn checked_name(&self, site: &UsageSite<'_>, report: &mut Diagnostics) -> Option<IndexName> {
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
                .at_opt(site.position()),
            );
            return None;
        };
        match IndexName::parse(raw) {
            Ok(name) => Some(name),
            Err(error) => {
                report.push(Diagnostic::error(error.to_string()).at_opt(site.key("name")));
                None
            }
        }
    }

    /// The publisher an entry declares.
    ///
    /// `None` is an error already reported; `Some(None)` is an `exempt`
    /// entry that declares none.
    fn checked_publisher(
        &self,
        site: &UsageSite<'_>,
        report: &mut Diagnostics,
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
            // Without a name the message can only say the key is missing.
            report.push(match name {
                Some(name) => Diagnostic::error(format!(
                    "usage `{name}` has no `publisher`; only a standard-library usage, whose \
                     `kpar-url` is an OMG standard-library IRI, may omit it"
                ))
                .at_opt(site.position()),
                None => missing_key(
                    "publisher",
                    &KeyLocation::ArrayTable(site.array.to_owned()),
                    None,
                )
                .at_opt(site.position()),
            });
            return None;
        };
        match IndexPublisher::parse(raw) {
            Ok(publisher) => Some(Some(publisher)),
            Err(error) => {
                report.push(Diagnostic::error(error.to_string()).at_opt(site.key("publisher")));
                None
            }
        }
    }

    fn checked_marker(
        &self,
        site: &UsageSite<'_>,
        report: &mut Diagnostics,
        allowed: bool,
        label: &str,
    ) -> Option<InheritFrom> {
        let marker = self.workspace.clone()?;
        if !allowed {
            // A group's own entries have nothing to inherit from.
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` sets a `workspace` marker, which only a project's \
                     `inline-usages` may do"
                ))
                .at_opt(site.key("workspace")),
            );
            return None;
        }
        match InheritFrom::try_from(marker) {
            Ok(from) => Some(from),
            Err(error) => {
                report.push(Diagnostic::error(error.to_string()).at_opt(site.key("workspace")));
                None
            }
        }
    }

    /// The group entry is used as-is, so an entry carrying a marker may
    /// give nothing but the identity.
    fn check_opt_in_is_bare(
        &self,
        site: &UsageSite<'_>,
        report: &mut Diagnostics,
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
                .at_opt(site.key(key)),
            );
        }
    }

    fn checked_kind(
        &self,
        site: &UsageSite<'_>,
        report: &mut Diagnostics,
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
                .at_opt(site.position()),
            );
            return None;
        }

        // Only the index kind carries a version requirement. The `kpar-url`
        // message says why: the IRI names one release.
        let reject_version = |report: &mut Diagnostics, kind: &str| {
            if self.version.is_some() {
                let message = if kind == "kpar-url" {
                    format!(
                        "usage `{label}` carries `version`, which a `kpar-url` usage does not \
                         take; the IRI names one release"
                    )
                } else {
                    format!(
                        "usage `{label}` carries `version`, which a `{kind}` usage does not take"
                    )
                };
                report.push(Diagnostic::error(message).at_opt(site.key("version")));
            }
        };

        let local_path = |report: &mut Diagnostics, key: &str, raw: &str| {
            if is_normal_local_path(raw) {
                return Some(Utf8UnixPathBuf::from(raw));
            }
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` sets `{key} = \"{raw}\"`, which is not in the form a \
                     local path takes:\n`/`-separated, with `..` only as leading components, \
                     no `.` component, no repeated or trailing slash and no backslash"
                ))
                .at_opt(site.key(key)),
            );
            None
        };

        if let Some(dir) = self.dir.as_deref() {
            reject_version(report, "dir");
            return local_path(report, "dir", dir).map(Kind::SourcePath);
        }
        if let Some(kpar_path) = self.kpar_path.as_deref() {
            reject_version(report, "kpar-path");
            return local_path(report, "kpar-path", kpar_path).map(Kind::KparPath);
        }
        if let Some(url) = self.url.as_deref() {
            reject_version(report, "kpar-url");
            let Ok(parsed) = Iri::parse(url.to_owned()) else {
                report.push(
                    Diagnostic::error(format!(
                        "usage `{label}` sets `kpar-url = \"{url}\"`, which is not a valid IRI"
                    ))
                    .at_opt(site.key("kpar-url")),
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
                .at_opt(site.position()),
            );
            return None;
        };
        let Ok(constraint) = semver::VersionReq::parse(version) else {
            report.push(
                Diagnostic::error(format!(
                    "usage `{label}` sets `version = \"{version}\"`, which is not a version \
                     requirement"
                ))
                .at_opt(site.key("version")),
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
    /// `None` only if a kind that requires a publisher has none, which
    /// validation rules out.
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

/// Two entries whose identities normalize to the same value are the same
/// project declared twice and are rejected, whether or not they agree on
/// kind or constraint. Each array is counted separately.
///
/// Every colliding pair is reported, at the later of the two.
pub fn check_duplicates<E: UsageEntry>(entries: &[(UsageSite<'_>, E)]) -> Vec<Diagnostic> {
    duplicate_pairs(entries, |(_, entry)| entry.normalized_identity())
        .into_iter()
        .map(|(first, later)| {
            let (site, entry) = &entries[later];
            Diagnostic::error(duplicate_message(&entries[first].1.label(), &entry.label()))
                .at_opt(site.position())
        })
        .collect()
}

/// The wording of the duplicate rule.
pub fn duplicate_message(first: &str, later: &str) -> String {
    format!(
        "usages `{first}` and `{later}` name the same project; two entries whose publisher and \
         name normalize to the same value are a duplicate"
    )
}

/// The pairs of entries whose identities are equal, as `(earlier,
/// later)` indices.
///
/// Every colliding pair is returned; positions are the caller's business.
pub fn duplicate_pairs<T, I: PartialEq>(
    items: &[T],
    identity: impl Fn(&T) -> I,
) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let wanted = identity(item);
        if let Some(first) = items[..index]
            .iter()
            .position(|earlier| identity(earlier) == wanted)
        {
            pairs.push((first, index));
        }
    }
    pairs
}

impl From<Usage> for UsageRaw {
    fn from(usage: Usage) -> Self {
        let mut raw = Self {
            // A standard-library usage has none to write.
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
                // None for an opt-in to a standard-library entry.
                publisher: publisher.map(|publisher| publisher.to_string()),
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

/// A `dir` or `kpar-path` is taken exactly as written, and admitted only
/// in the one form Sysand writes: `/`-separated, with no backslash or
/// NUL, no `.` component, `..` only as the leading components of a
/// relative path, and no empty component (so no repeated or trailing
/// slash). Unlike `sources` and `members`, such a path may leave the
/// directory it is relative to. An absolute path is admitted, and held to
/// the same rules past its root.
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

#[cfg(test)]
#[path = "./usage_tests.rs"]
mod tests;
