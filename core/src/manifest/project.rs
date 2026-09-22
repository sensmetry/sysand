// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The `[project]` table of a `model.toml` as it is written --
//! inheritance markers included -- and [`ResolvedProject`], what that
//! metadata becomes once the markers have been followed and the
//! metamodel-dependent validation has run.
//!
//! The two are separate because model-toml.md "Reading and validating"
//! keeps them apart: "required-key checks that depend on the metamodel
//! (`sources`) run after inheritance has been resolved, since `metamodel`
//! may itself be inherited from a workspace group. `manifest-version`
//! (read first) and the keys required by every metamodel (`publisher`,
//! `name`, `version`) are checked before that."
//!
//! So [`ProjectTable::validate`] checks everything a manifest can be
//! judged on by itself, and [`ProjectManifest::resolve`] -- which takes
//! the workspace the project is a *member* of, or `None` -- checks what
//! is left.

use fluent_uri::Iri;
use semver::Version;
use spdx::Expression;

use typed_path::Utf8UnixPathBuf;

use crate::manifest::common::{
    InheritFrom, Maintainer, NormalizedIdentity, ResolvedUrls, Sources, StdlibStem,
    admitted_source, normalized_identity, root_path,
};
use crate::manifest::diagnostics::{
    Diagnostic, KeyLocation, Position, ReadWarning, Report, Validated, missing_key,
};
use crate::manifest::inherit::{
    MetaProperty, from_group, inheritable, inheritable_table, own, resolve_optional,
    resolve_property,
};
use crate::manifest::reader::{self, TableReader, checked, describe, maintainers, parsed};
use crate::manifest::spans::Positions;
use crate::manifest::usage::{
    LocatedUsage, MemberUsage, Usage, UsageRaw, UsageSite, check_duplicates,
};
use crate::manifest::workspace_table::{GroupProperties, WorkspaceTable};
use crate::model::{IndexName, IndexPublisher, KnownMetamodel, MetamodelFamily};

/// Every key model-toml.md "Main project metadata" documents, in the
/// order it documents them, which is the order a generated file writes.
pub(crate) const PROJECT_KEYS: &[&str] = &[
    "publisher",
    "name",
    "version",
    "sources",
    "license",
    "description",
    "topics",
    "metamodel",
    "maintainers",
    "stdlib",
    "root",
    "urls",
];

/// model-toml.md `urls`: "currently allowed keys: website, repository,
/// documentation".
pub(crate) const URL_KEYS: &[&str] = &["website", "documentation", "repository"];

/// How this table names itself in a message and addresses itself in the
/// span index.
const TABLE: &str = "project";

/// The `urls` table of a `[project]`, as written.
///
/// workspaces.md "Common metadata" lists `urls` as inheritable "(whole
/// table or individual keys can be inherited)", so both levels are a
/// [`MetaProperty`]; that the two cannot be combined is the overlap rule
/// of [`crate::manifest::inherit::inheritable_table`].
#[derive(Clone, Debug, Default)]
pub struct Urls {
    pub website: Option<MetaProperty<Iri<String>>>,
    pub documentation: Option<MetaProperty<Iri<String>>>,
    pub repository: Option<MetaProperty<Iri<String>>>,
}

/// The `[project]` table as the file writes it.
///
/// `spdx::Expression` has no `PartialEq`, so this type derives only
/// `Clone` and `Debug` -- as [`WorkspaceTable`] does, for the same
/// reason.
#[derive(Clone, Debug)]
pub struct ProjectTable {
    pub name: IndexName,
    pub publisher: MetaProperty<IndexPublisher>,
    pub version: MetaProperty<Version>,
    /// Never inherited, and required by the OMG metamodel -- which is
    /// checked in [`ProjectManifest::resolve`], since the metamodel may
    /// itself be inherited.
    pub sources: Option<Sources>,
    pub license: Option<MetaProperty<Expression>>,
    pub description: Option<String>,
    pub topics: Option<MetaProperty<Vec<String>>>,
    pub metamodel: Option<MetaProperty<KnownMetamodel>>,
    pub maintainers: Option<MetaProperty<Vec<Maintainer>>>,
    /// model-toml.md "Standard library": not inheritable, so a plain
    /// value.
    pub stdlib: Option<StdlibStem>,
    /// `root`, **exactly as written and unjudged**. It belongs to the
    /// Syside schema (plan-phase2.md WP-A1, `phase2-a0a-memo.md` A-1(b)):
    /// under that family it is a required key naming one of the
    /// `sources` entries, and under any other it is a foreign key that
    /// costs a warning and nothing else, whatever its value. Which of the
    /// two applies depends on the family, and the family may arrive by
    /// marker, so the reader keeps the value and
    /// [`ProjectManifest::resolve`] judges it -- and a writer writes it
    /// back as it was.
    pub root: Option<toml::Value>,
    pub urls: Option<MetaProperty<Urls>>,
    /// Where each `sources` entry is, in the order they were written,
    /// for the check of each entry against the metamodel -- which
    /// likewise waits for the metamodel, by which time the reader that could point at an entry
    /// is long gone.
    sources_at: Vec<Option<Position>>,
    /// Where `stdlib` is, for the refusal that can only be decided once
    /// the metamodel is known.
    stdlib_at: Option<Position>,
    /// Where the table itself is, for the missing-`sources` error, which
    /// likewise waits for the metamodel.
    table_at: Option<Position>,
    /// Where `root` is, for every judgement of it, all of which wait for
    /// the metamodel.
    root_at: Option<Position>,
}

/// A `model.toml` that defines a project: the `[project]` table plus the
/// top-level `inline-usages` array.
#[derive(Clone, Debug)]
pub struct ProjectManifest {
    pub project: ProjectTable,
    pub inline_usages: Vec<MemberUsage>,
}

/// `[project]` metadata with every marker followed and the
/// metamodel-dependent validation done. This is what commands work with.
#[derive(Clone, Debug)]
pub struct ResolvedProject {
    pub name: IndexName,
    pub publisher: IndexPublisher,
    pub version: Version,
    /// Carried straight over: `sources` is never inherited.
    pub sources: Sources,
    pub license: Option<Expression>,
    pub description: Option<String>,
    pub topics: Vec<String>,
    /// model-toml.md "Metamodel": mandatory in a `model.toml`, and one
    /// of exactly four values, so a resolved project always knows its
    /// family, spec kind and release date.
    pub metamodel: KnownMetamodel,
    pub maintainers: Vec<Maintainer>,
    pub stdlib: Option<StdlibStem>,
    /// The Syside family's entry point: one of the `sources` entries, as
    /// written. Always `Some` for a Syside project, which requires it,
    /// and always `None` for an OMG one, which "is built from `sources`"
    /// whether or not the key was written.
    pub root: Option<Utf8UnixPathBuf>,
    pub urls: ResolvedUrls,
    /// Each usage with the directory its paths are relative to: the
    /// project's own entries against the project, inherited ones against
    /// the workspace root.
    pub usages: Vec<LocatedUsage>,
}

impl ResolvedProject {
    pub fn identity(&self) -> NormalizedIdentity {
        normalized_identity(&self.publisher, &self.name)
    }
}

impl ProjectTable {
    /// A `[project]` table built from values rather than read from a
    /// file: the three keys every metamodel requires, and nothing else.
    ///
    /// This is where a command that *writes* a manifest starts one.
    /// Every other key is public, so the caller sets
    /// what it has; a constructor taking all twelve would say no more
    /// and read far worse.
    ///
    /// `publisher` and `version` are wrapped as
    /// [`MetaProperty::Provided`]: a generated file gives its values
    /// rather than inheriting them. A caller that wants a marker assigns
    /// the field.
    ///
    /// The returned table has **no `sources`**, which for the OMG
    /// metamodel is not a manifest that resolves -- "a project with no
    /// source files writes `sources = []`", which is
    /// `Some(Sources::default())`, not `None`. It is still renderable:
    /// what [`crate::manifest::edit::render_new`] writes is what the
    /// table holds.
    ///
    /// Neither of the two private position fields can be set from
    /// outside, so the diagnostics a built manifest raises in
    /// [`ProjectManifest::resolve`] are unpositioned. There is no file
    /// for them to point into.
    pub fn new(publisher: IndexPublisher, name: IndexName, version: Version) -> Self {
        Self {
            name,
            publisher: MetaProperty::Provided(publisher),
            version: MetaProperty::Provided(version),
            sources: None,
            license: None,
            description: None,
            topics: None,
            metamodel: None,
            maintainers: None,
            stdlib: None,
            root: None,
            urls: None,
            sources_at: Vec::new(),
            stdlib_at: None,
            table_at: None,
            root_at: None,
        }
    }

    /// Read and check the contents of a `[project]` table.
    ///
    /// Every key is read, and every problem pushed, before the value is
    /// built: a missing required key must not cost the report everything
    /// found after it.
    pub fn validate(
        table: &toml::Table,
        positions: &Positions<'_>,
    ) -> Result<Validated<Self>, Report> {
        let reader = TableReader::new(
            table,
            vec![TABLE.to_owned()],
            KeyLocation::Table(TABLE.to_owned()),
            positions,
        );
        let mut report = Report::new();

        // Read first: the metamodel selects the schema the rest of the
        // table is judged against (model-toml.md "Metamodel").
        let metamodel = inheritable(&reader, "metamodel", &mut report, |report| {
            reader::metamodel(&reader, report)
        });
        let name = own(
            &reader,
            "name",
            "it identifies one project",
            &mut report,
            |report| {
                let raw = reader.string("name", report)?;
                parsed(IndexName::parse(raw), &reader, "name", report)
            },
        );
        let publisher = inheritable(&reader, "publisher", &mut report, |report| {
            let raw = reader.string("publisher", report)?;
            parsed(IndexPublisher::parse(raw), &reader, "publisher", report)
        });
        let version = inheritable(&reader, "version", &mut report, |report| {
            let raw = reader.string("version", report)?;
            checked(
                Version::parse(raw),
                &reader,
                "version",
                raw,
                "a valid version",
                report,
            )
        });
        let sources = own(
            &reader,
            "sources",
            "every member writes out its own source files in full",
            &mut report,
            |report| sources(&reader, report),
        );
        let (sources, sources_at) = match sources {
            Some((sources, at)) => (Some(sources), at),
            None => (None, Vec::new()),
        };
        let license = inheritable(&reader, "license", &mut report, |report| {
            let raw = reader.string("license", report)?;
            checked(
                Expression::parse(raw),
                &reader,
                "license",
                raw,
                "a valid SPDX expression",
                report,
            )
        });
        let description = own(
            &reader,
            "description",
            "it identifies one project",
            &mut report,
            |report| reader.string("description", report).map(ToOwned::to_owned),
        );
        let topics = inheritable(&reader, "topics", &mut report, |report| {
            reader
                .strings("topics", report)
                .map(|values| values.into_iter().map(ToOwned::to_owned).collect())
        });
        let maintainers = inheritable(&reader, "maintainers", &mut report, |report| {
            maintainers(&reader, report)
        });
        let stdlib = own(
            &reader,
            "stdlib",
            "it is a per-project identifier",
            &mut report,
            |report| {
                let raw = reader.string("stdlib", report)?;
                parsed(StdlibStem::parse(raw), &reader, "stdlib", report)
            },
        );
        // Read for the marker only (a marker is an error); the value is
        // kept as it is and judged in `resolve`, where the family is
        // known. Nothing about `root` -- not its type, not its shape --
        // can be said before then: under an OMG family it is a foreign
        // key that is ignored whatever it holds.
        let root = own(
            &reader,
            "root",
            "it is a per-project entry point",
            &mut report,
            |_| reader.raw().get("root").cloned(),
        );
        let urls = inheritable_table(&reader, "urls", &mut report, |report| urls(&reader, report));

        for (key, present) in [
            ("publisher", publisher.is_some()),
            ("name", name.is_some()),
            ("version", version.is_some()),
        ] {
            // A key that is written but could not be read has already
            // been reported; saying it is missing as well would be two
            // complaints about one mistake.
            if !present && !reader.has(key) {
                report.push(missing_key(key, reader.location(), None).at_opt(reader.here()));
            }
        }

        // Rule (a)'s suffix -- "this may be because the project uses a
        // metamodel this version of Sysand does not support" -- is for
        // json projects only. A `model.toml` whose metamodel is
        // unrecognized never reaches this line: it was refused above.
        report.extend(reader.unknown_keys(PROJECT_KEYS, None));
        report.sort_by_position();

        // The value is built only once every key has been read, so that
        // a missing one costs the report nothing found after it.
        match (name, publisher, version) {
            (Some(name), Some(publisher), Some(version)) => report.into_result(Self {
                name,
                publisher,
                version,
                sources,
                license,
                description,
                topics,
                metamodel,
                maintainers,
                stdlib,
                root,
                urls,
                sources_at,
                stdlib_at: reader.at("stdlib"),
                table_at: reader.here(),
                root_at: reader.at("root"),
            }),
            _ => Err(report),
        }
    }
}

/// model-toml.md `sources`. The rules are [`Sources::new`]'s; what is
/// added here is where each complaint points.
fn sources(
    reader: &TableReader<'_>,
    report: &mut Report,
) -> Option<(Sources, Vec<Option<Position>>)> {
    let raw = reader.strings("sources", report)?;
    let paths = raw
        .iter()
        .map(|entry| typed_path::Utf8UnixPathBuf::from(*entry))
        .collect();
    match Sources::check(paths) {
        Ok(sources) => {
            let at = (0..raw.len())
                .map(|index| reader.at_element("sources", index))
                .collect();
            Some((sources, at))
        }
        Err(problems) => {
            for (index, error) in problems {
                report.push(
                    Diagnostic::error(error.to_string())
                        .at_opt(reader.at_element("sources", index)),
                );
            }
            None
        }
    }
}

fn urls(reader: &TableReader<'_>, report: &mut Report) -> Option<Urls> {
    let urls = reader.table("urls", report)?;
    let one = |key: &str, report: &mut Report| {
        inheritable(&urls, key, report, |report| {
            let raw = urls.string(key, report)?;
            checked(
                Iri::parse(raw.to_owned()).map_err(|(error, _)| error),
                &urls,
                key,
                raw,
                "a valid URL",
                report,
            )
        })
    };
    let website = one("website", report);
    let documentation = one("documentation", report);
    let repository = one("repository", report);
    report.extend(urls.unknown_keys(URL_KEYS, None));
    Some(Urls {
        website,
        documentation,
        repository,
    })
}

impl ProjectManifest {
    /// Read the project side of a document: the `[project]` table and the
    /// top-level `inline-usages` array.
    ///
    /// Which tables the document is allowed to have, and the warning for
    /// an unknown *top-level* key, belong to the reader of the file as a
    /// whole. An absent `[project]` is therefore not diagnosed here: it
    /// comes back as the three required keys missing, and a file with no
    /// project at all is refused earlier.
    pub fn validate(
        document: &toml::Table,
        positions: &Positions<'_>,
    ) -> Result<Validated<Self>, Report> {
        let mut report = Report::new();
        let top = TableReader::new(document, Vec::new(), KeyLocation::TopLevel, positions);

        // An absent `[project]` is an empty one: the three required
        // keys are then reported missing, which is what a file with no
        // project side has wrong with it.
        //
        // A `project` that is *written* but is not a table is a
        // different thing: `TableReader::table` has already said so, and
        // reading an empty table on top of that would add three
        // missing-key errors for one mistake.
        let empty = toml::Table::new();
        let table = top.table(TABLE, &mut report);
        let malformed = table.is_none() && top.has(TABLE);
        let project = if malformed {
            None
        } else {
            let table = table.as_ref().map_or(&empty, TableReader::raw);
            match ProjectTable::validate(table, positions) {
                Ok(validated) => {
                    let (value, warnings) = validated.into_parts();
                    report.extend(warnings);
                    Some(value)
                }
                Err(problems) => {
                    report.extend(problems.into_diagnostics());
                    None
                }
            }
        };

        let mut entries = Vec::new();
        for (index, entry) in top.tables("inline-usages", &mut report).iter().enumerate() {
            let site = UsageSite {
                array: "inline-usages",
                index,
            };
            match UsageRaw::from_table(entry.raw()).validate_member(&site, positions) {
                Ok(usage) => entries.push((site, usage)),
                Err(problems) => report.extend(problems.into_diagnostics()),
            }
        }
        // model-toml.md "Normalization": duplicates are counted per
        // array, and only the entries that validated can be compared.
        report.extend(check_duplicates(&entries, positions));

        report.sort_by_position();
        match project {
            Some(project) => report.into_result(Self {
                project,
                inline_usages: entries.into_iter().map(|(_, usage)| usage).collect(),
            }),
            None => Err(report),
        }
    }

    /// Whether anything in this manifest is inherited.
    ///
    /// workspaces.md: markers "do not establish membership, but a project
    /// carrying one cannot be read outside a workspace", while "a `dir`
    /// usage to a marker-free non-member reads it as an ordinary
    /// standalone project". That is the one question a caller has to ask
    /// before resolving rather than by catching an error.
    pub fn inherits(&self) -> bool {
        let project = &self.project;
        project.publisher.inherited().is_some()
            || project.version.inherited().is_some()
            || marked(project.license.as_ref())
            || marked(project.topics.as_ref())
            || marked(project.metamodel.as_ref())
            || marked(project.maintainers.as_ref())
            || project.urls.as_ref().is_some_and(|urls| match urls {
                MetaProperty::Inherited(_) => true,
                MetaProperty::Provided(urls) => {
                    marked(urls.website.as_ref())
                        || marked(urls.documentation.as_ref())
                        || marked(urls.repository.as_ref())
                }
            })
            || self.inline_usages.iter().any(|usage| {
                matches!(
                    usage,
                    MemberUsage::Inherited {
                        publisher: _,
                        name: _,
                        from: _,
                        at: _
                    }
                )
            })
    }

    /// workspaces.md "Explicit inheritance" and "Common usages": follow
    /// every marker, then run the validation that needs the effective
    /// metamodel.
    ///
    /// `workspace` is the table of the workspace this project is a
    /// *member* of, and `None` for every other project: "Markers resolve
    /// only against the workspace the project is a member of. A project
    /// that is not a member is outside every workspace regardless of
    /// where its directory sits."
    ///
    /// Every marker is followed before the two checks that need the
    /// result, so a project with three failing markers reports three.
    pub fn resolve(
        self,
        workspace: Option<&WorkspaceTable>,
    ) -> Result<Validated<ResolvedProject>, Report> {
        let mut report = Report::new();
        let Self {
            project,
            inline_usages,
        } = self;
        let ProjectTable {
            name,
            publisher,
            version,
            sources,
            license,
            description,
            topics,
            metamodel,
            maintainers,
            stdlib,
            root,
            urls,
            sources_at,
            stdlib_at,
            table_at,
            root_at,
        } = project;

        let publisher = collect(
            resolve_property(
                publisher,
                "publisher",
                workspace,
                |group| group.publisher.clone(),
                |value| value,
            ),
            &mut report,
        );
        let version = collect(
            resolve_property(
                version,
                "version",
                workspace,
                |group| group.version.clone(),
                |value| value,
            ),
            &mut report,
        );
        let license = collect(
            resolve_optional(
                license,
                "license",
                workspace,
                |group| group.license.clone(),
                |value| value,
            ),
            &mut report,
        )
        .flatten();
        let topics = collect(
            resolve_optional(
                topics,
                "topics",
                workspace,
                |group| group.topics.clone(),
                |value| value,
            ),
            &mut report,
        )
        .flatten()
        .unwrap_or_default();
        let maintainers = collect(
            resolve_optional(
                maintainers,
                "maintainers",
                workspace,
                |group| group.maintainers.clone(),
                |value| value,
            ),
            &mut report,
        )
        .flatten()
        .unwrap_or_default();
        // The outer `Option` is whether the key resolved at all, which is
        // not the same as an absent `metamodel`: the checks below are
        // skipped entirely when it did not.
        let metamodel = collect(
            resolve_optional(
                metamodel,
                "metamodel",
                workspace,
                |group| group.metamodel.clone(),
                |value| value,
            ),
            &mut report,
        );
        // model-toml.md "Metamodel": "`metamodel` is a URL, and in a
        // `model.toml` it is mandatory." Checked after inheritance,
        // because a marker satisfies the requirement like any other
        // value. Declared and group values alike were checked against
        // the four when their tables were read.
        let metamodel = match metamodel {
            None => None,
            Some(None) => {
                report.push(
                    Diagnostic::error(format!(
                        "missing key `metamodel` in {};\nevery project declares one of {list}",
                        KeyLocation::Table(TABLE.to_owned()),
                        list = KnownMetamodel::RECOGNIZED,
                    ))
                    .at_opt(table_at.clone()),
                );
                None
            }
            Some(Some(value)) => Some(value),
        };
        let urls = collect(resolve_urls(urls, workspace), &mut report);

        let mut usages = Vec::with_capacity(inline_usages.len());
        for entry in inline_usages {
            match entry {
                MemberUsage::Declared(usage) => usages.push(LocatedUsage::declared(usage)),
                MemberUsage::Inherited {
                    publisher,
                    name,
                    from,
                    at,
                } => match from_group_usage(workspace, publisher.as_ref(), &name, &from, at) {
                    Ok(usage) => usages.push(LocatedUsage::inherited(usage)),
                    Err(problem) => report.push(problem),
                },
            }
        }

        // `root`, judged now that the family is known (plan-phase2.md
        // WP-A1; `phase2-a0a-memo.md` A-1(b)). Phase 1 judged it against
        // the *declared* metamodel and so warned a member whose Syside
        // metamodel arrived by marker about its own `root`.
        let location = KeyLocation::Table(TABLE.to_owned());
        let family = metamodel.as_ref().map(|known| known.family);
        let root_written = root.is_some();
        let root = match (root, family) {
            (None, _) => None,
            // The key's own family: a string, in the form a `sources`
            // entry takes.
            (Some(toml::Value::String(raw)), Some(MetamodelFamily::Syside)) => {
                match root_path(&raw) {
                    Ok(path) => Some(path),
                    Err(problem) => {
                        report.push(Diagnostic::error(problem.to_string()).at_opt(root_at.clone()));
                        None
                    }
                }
            }
            (Some(other), Some(MetamodelFamily::Syside)) => {
                report.push(
                    Diagnostic::error(format!(
                        "`root` in {location} must be a string, but it is {}",
                        describe(&other)
                    ))
                    .at_opt(root_at.clone()),
                );
                None
            }
            // model-toml.md "Metamodel", rule (b): an OMG project that
            // carries both `sources` and `root` gets only the warning and
            // is built from `sources`, whatever the value
            // (design/syside-metamodel.md, Reading and validating).
            (Some(_), Some(MetamodelFamily::Omg)) => {
                report.push(
                    Diagnostic::from(ReadWarning::ForeignSchemaKey {
                        location: location.clone(),
                        key: "root".to_owned(),
                        supported_by: MetamodelFamily::Syside,
                    })
                    .at_opt(root_at.clone()),
                );
                None
            }
            // No family to judge against: the metamodel is missing or
            // failed to resolve, and that is already in the report. A
            // metamodel that says nothing about the schema decides
            // nothing about `root`.
            (Some(_), None) => None,
        };

        // model-toml.md: `sources` is "always required for the OMG
        // metamodel", and under `phase2-a0a-memo.md` A-1(b) it is the
        // Syside family's membership list too; what differs is the text
        // (phase 1's appendix A row 18 for OMG, appendix B row P2-23 for
        // Syside).
        //
        // A project whose `metamodel` is missing entirely still gets
        // this: every one of the four requires `sources`, so the plain
        // missing-key text is right without a metamodel to name, and one
        // report holds both problems.
        let sources = sources.or_else(|| {
            report.push(missing_sources(metamodel.as_ref()).at_opt(table_at.clone()));
            None
        });

        let syside = family == Some(MetamodelFamily::Syside);
        // `root` is required of every Syside project. A `root` that was
        // written and refused above is not also missing.
        if syside && !root_written {
            report.push(
                Diagnostic::error(format!(
                    "missing key `root` in {location}; the Syside metamodel requires it"
                ))
                .at_opt(table_at.clone()),
            );
        }
        // `root` names one of the `sources` entries, compared as written
        // -- the entries themselves are literal (model-toml.md `sources`:
        // "Two equal entries are a duplicate", equal, not equivalent).
        // With no `sources` at all there is no list to be absent from,
        // and the missing `sources` has been reported.
        if syside
            && let (Some(root), Some(sources)) = (root.as_ref(), sources.as_ref())
            && !sources.iter().any(|entry| entry == root)
        {
            report.push(
                Diagnostic::error(format!(
                    "`root` is `{root}`, which is not listed in `sources`"
                ))
                .at_opt(root_at),
            );
        }

        // model-toml.md "Metamodel": "The spec kind
        // bounds what `sources` may list". Here and not in
        // `Sources::new`, because the metamodel may itself be inherited
        // and a table reader cannot know it; the entry's position was
        // kept for exactly this.
        //
        // This is also where a Syside `root` with an extension the
        // metamodel does not admit is refused: `root` is one of these
        // entries, so this check covers it and `root` raises nothing of
        // its own for the extension -- one file, one complaint.
        if let (Some(sources), Some(kind)) = (sources.as_ref(), metamodel.as_ref()) {
            for (index, entry) in sources.iter().enumerate() {
                if let Err(problem) = admitted_source(entry, kind) {
                    report.push(
                        problem.at_opt(
                            sources_at
                                .get(index)
                                .cloned()
                                .flatten()
                                .or_else(|| table_at.clone()),
                        ),
                    );
                }
            }
        }

        // model-toml.md "Standard library": the key "is refused unless
        // the project's publisher normalizes to `omg` and its metamodel,
        // declared or inherited, is a recognized OMG one (the IRI needs
        // its release date)".
        if stdlib.is_some()
            && let Some(publisher) = publisher.as_ref()
            && let Some(kind) = metamodel.as_ref()
            && (!publisher.is_stdlib_publisher() || kind.family != MetamodelFamily::Omg)
        {
            report.push(
                Diagnostic::error(format!(
                    "`stdlib` is only allowed for the standard library itself: a publisher \
                     normalizing to `omg` and an OMG metamodel;\n\
                     found publisher `{publisher}` and {}",
                    kind.describe()
                ))
                .at_opt(stdlib_at),
            );
        }

        report.sort_by_position();
        match (publisher, version, sources, metamodel, urls) {
            (Some(publisher), Some(version), Some(sources), Some(metamodel), Some(urls)) => report
                .into_result(ResolvedProject {
                    name,
                    publisher,
                    version,
                    sources,
                    license,
                    description,
                    topics,
                    metamodel,
                    maintainers,
                    stdlib,
                    root,
                    urls,
                    usages,
                }),
            _ => Err(report),
        }
    }
}

/// Whether an optional key carries a marker.
fn marked<T>(property: Option<&MetaProperty<T>>) -> bool {
    property.is_some_and(|property| property.inherited().is_some())
}

fn collect<T>(result: Result<T, Diagnostic>, report: &mut Report) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(problem) => {
            report.push(problem);
            None
        }
    }
}

fn resolve_urls(
    urls: Option<MetaProperty<Urls>>,
    workspace: Option<&WorkspaceTable>,
) -> Result<ResolvedUrls, Diagnostic> {
    let url = |get: fn(&ResolvedUrls) -> &Option<Iri<String>>| {
        move |group: &GroupProperties| {
            let urls = group.urls.as_ref()?;
            get(urls).clone()
        }
    };
    match urls {
        None => Ok(ResolvedUrls::default()),
        // Whole-table inheritance takes the group's `urls` as it is.
        Some(MetaProperty::Inherited(inherited)) => {
            from_group(&inherited, "urls", workspace, |group| group.urls.clone())
        }
        Some(MetaProperty::Provided(urls)) => Ok(ResolvedUrls {
            website: resolve_optional(
                urls.website,
                "urls.website",
                workspace,
                url(|urls| &urls.website),
                |value| value,
            )?,
            documentation: resolve_optional(
                urls.documentation,
                "urls.documentation",
                workspace,
                url(|urls| &urls.documentation),
                |value| value,
            )?,
            repository: resolve_optional(
                urls.repository,
                "urls.repository",
                workspace,
                url(|urls| &urls.repository),
                |value| value,
            )?,
        }),
    }
}

/// workspaces.md "Common usages": an entry that gives only the identity
/// plus a marker "takes the matching group entry whole". Entries "are
/// matched by publisher and name, compared literally", so an opt-in has
/// to spell both exactly as the group entry does -- including declaring
/// no publisher, which for a standard-library entry is the spelling:
/// "an opt-in to such an entry likewise gives only `name` and the
/// marker".
fn from_group_usage(
    workspace: Option<&WorkspaceTable>,
    publisher: Option<&IndexPublisher>,
    name: &IndexName,
    from: &InheritFrom,
    at: Option<Position>,
) -> Result<Usage, Diagnostic> {
    let label = match publisher {
        Some(publisher) => format!("usage `{publisher}/{name}`"),
        None => format!("usage `{name}`"),
    };
    let Some(workspace) = workspace else {
        return Err(Diagnostic::error(format!(
            "{label} inherits from {from}, but the project is not a member of any workspace"
        ))
        .at_opt(at));
    };
    let Some(group) = workspace.group_properties(from) else {
        return Err(Diagnostic::error(format!(
            "{label} inherits from {from}, which the workspace does not define"
        ))
        .at_opt(at));
    };
    group
        .inline_usages
        .iter()
        .find(|usage| {
            // Compared literally, and absence is part of the spelling:
            // a group entry that declares no publisher is a
            // standard-library usage, and only an opt-in that likewise
            // declares none names it. The two mismatched cases fall
            // through to "no entry with that publisher and name", which
            // is the truth.
            let publisher_matches = match (publisher, usage.publisher()) {
                (Some(wanted), Some(declared)) => wanted.as_str() == declared.as_str(),
                (None, None) => true,
                _ => false,
            };
            publisher_matches && usage.name().as_str() == name.as_str()
        })
        .cloned()
        .ok_or_else(|| {
            Diagnostic::error(format!(
                "{label} inherits from {from}, which has no entry with that publisher and name"
            ))
            .at_opt(at)
        })
}

/// Phase 1's appendix A row 18 and its siblings, one per family. The
/// Syside text is plan-phase2.md appendix B row P2-23, which supersedes
/// row 18's "whose schema this version of Sysand does not support": the
/// schema is supported, and needs `sources` like the OMG one
/// (`phase2-a0a-memo.md` A-1(b)).
fn missing_sources(metamodel: Option<&KnownMetamodel>) -> Diagnostic {
    let location = KeyLocation::Table(TABLE.to_owned());
    match metamodel {
        Some(kind) if kind.family == MetamodelFamily::Syside => Diagnostic::error(format!(
            "missing key `sources` in {location}; the Syside metamodel requires it\n\
             (`root` names one of its entries)"
        )),
        // No metamodel to name -- it is missing too, and said so -- but
        // every one of the four requires `sources` in v1, so the key is
        // missing whichever one this project turns out to declare.
        None => missing_key("sources", &location, None),
        Some(_) => Diagnostic::error(format!(
            "missing key `sources` in {location}; the OMG metamodel requires it\n\
             (a project with no source files writes `sources = []`)"
        )),
    }
}

#[cfg(test)]
#[path = "./project_tests.rs"]
mod tests;
