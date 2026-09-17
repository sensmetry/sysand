// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use indexmap::IndexMap;

use super::{MigrateOptions, migrate_project};
use crate::{
    manifest::{ModelToml, project::ProjectManifest},
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsageRaw,
    },
};

/// The lookup for tests that have no `file:` usages: reaching it is a
/// test's own bug.
fn no_targets(resource: &str) -> Result<crate::manifest::usage::Usage, String> {
    panic!("no test fixture should resolve a target, but `{resource}` was looked up")
}

fn info(name: &str, publisher: Option<&str>, version: &str) -> InterchangeProjectInfoRaw {
    InterchangeProjectInfoRaw {
        name: name.to_owned(),
        publisher: publisher.map(ToOwned::to_owned),
        description: None,
        version: version.to_owned(),
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    }
}

/// The metamodel the shared fixture declares. A json project that has
/// none cannot be migrated without `--metamodel`,
/// so every test that is not about that says which one it means here.
const SYSML: &str = "https://www.omg.org/spec/SysML/20250201";

fn meta() -> InterchangeProjectMetadataRaw {
    InterchangeProjectMetadataRaw {
        index: IndexMap::new(),
        created: "2001-02-03T04:05:06Z".to_owned(),
        metamodel: Some(SYSML.to_owned()),
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    }
}

fn checksum_of(paths: &[&str]) -> IndexMap<String, InterchangeProjectChecksumRaw> {
    {
        paths
            .iter()
            .map(|path| {
                (
                    (*path).to_owned(),
                    InterchangeProjectChecksumRaw {
                        value: "0".repeat(64),
                        algorithm: "SHA256".to_owned(),
                    },
                )
            })
            .collect()
    }
}

fn migrated(
    info: &InterchangeProjectInfoRaw,
    meta: &InterchangeProjectMetadataRaw,
) -> ProjectManifest {
    let validated = migrate_project(info, meta, &MigrateOptions::default(), &no_targets)
        .unwrap_or_else(|report| panic!("the conversion succeeds:\n{report}"));
    match validated.value {
        ModelToml::Project(project) => *project,
        ModelToml::Workspace(_) => panic!("migrate produces a project"),
    }
}

fn errors(
    info: &InterchangeProjectInfoRaw,
    meta: &InterchangeProjectMetadataRaw,
    options: &MigrateOptions,
) -> String {
    migrate_project(info, meta, options, &no_targets)
        .expect_err("the conversion fails")
        .to_string()
}

// ------------------------------------------------- the carried fields

#[test]
fn a_minimal_project_migrates() {
    let manifest = migrated(&info("widgets", Some("acme"), "1.0.0"), &meta());
    assert_eq!(manifest.project.name.as_str(), "widgets");
    assert_eq!(
        manifest.project.sources.as_ref().map(Sources::is_empty),
        Some(true),
        "a project with no sources is written as `sources = []`"
    );
}

use crate::manifest::common::Sources;

#[test]
fn the_scalar_fields_are_carried() {
    let mut raw = info("widgets", Some("acme"), "1.2.3");
    raw.description = Some("a description".to_owned());
    raw.license = Some("MIT".to_owned());
    raw.maintainer = vec!["Ada <ada@example.com>".to_owned()];
    raw.website = Some("https://example.com/widgets".to_owned());
    raw.topic = vec!["systems".to_owned()];

    let manifest = migrated(&raw, &meta());
    assert_eq!(
        manifest.project.description.as_deref(),
        Some("a description")
    );
    assert_eq!(
        manifest
            .project
            .license
            .as_ref()
            .map(|l| l.provided().unwrap().to_string()),
        Some("MIT".to_owned())
    );
    assert_eq!(
        manifest
            .project
            .maintainers
            .as_ref()
            .unwrap()
            .provided()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        manifest
            .project
            .topics
            .as_ref()
            .unwrap()
            .provided()
            .unwrap(),
        &vec!["systems".to_owned()]
    );
    let urls = manifest.project.urls.as_ref().unwrap().provided().unwrap();
    assert_eq!(
        urls.website.as_ref().unwrap().provided().unwrap().as_str(),
        "https://example.com/widgets"
    );
}

#[test]
fn a_value_that_never_had_to_satisfy_the_manifests_rules_is_an_error() {
    // Legacy json names are unconstrained strings; a manifest name must
    // normalize to a valid purl part, which `ab` is too short for.
    let rendered = errors(
        &info("ab", Some("acme"), "1.0.0"),
        &meta(),
        &MigrateOptions::default(),
    );
    assert!(rendered.contains("`name`"), "{rendered}");
}

#[test]
fn a_non_semver_version_is_an_error() {
    let rendered = errors(
        &info("widgets", Some("acme"), "2023-11"),
        &meta(),
        &MigrateOptions::default(),
    );
    assert!(rendered.contains("not a semantic version"), "{rendered}");
}

#[test]
fn a_bad_license_maintainer_and_website_are_errors() {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.license = Some("Not A Licence".to_owned());
    raw.maintainer = vec!["Ada".to_owned()];
    raw.website = Some("not an iri".to_owned());
    let rendered = errors(&raw, &meta(), &MigrateOptions::default());
    assert!(rendered.contains("SPDX"), "{rendered}");
    assert!(rendered.contains("maintainer"), "{rendered}");
    assert!(rendered.contains("website"), "{rendered}");
}

// ------------------------------------------------------- the metamodel

fn with_metamodel(declared: Option<&str>) -> InterchangeProjectMetadataRaw {
    let mut m = meta();
    m.metamodel = declared.map(ToOwned::to_owned);
    m
}

fn supplying(metamodel: &str) -> MigrateOptions {
    MigrateOptions {
        metamodel: Some(metamodel.to_owned()),
        ..MigrateOptions::default()
    }
}

fn declared_metamodel(manifest: &ModelToml) -> Option<String> {
    manifest
        .as_project()
        .expect("a project")
        .project
        .metamodel
        .as_ref()
        .and_then(|property| property.provided())
        .map(ToString::to_string)
}

/// The value the json side declares is not one
/// `model.toml` can hold, and nothing was passed to replace it.
#[test]
fn an_unrecognized_metamodel_is_refused() {
    let rendered = errors(
        &info("widgets", Some("acme"), "1.0.0"),
        &with_metamodel(Some("https://example.com/spec/SysML/20251201")),
        &MigrateOptions::default(),
    );
    assert!(
        rendered.contains("declares `https://example.com/spec/SysML/20251201`"),
        "{rendered}"
    );
    assert!(rendered.contains("pass `--metamodel <url>`"), "{rendered}");
}

/// No value at all: `model.toml` requires the key and the json side
/// has none.
#[test]
fn a_project_with_no_metamodel_is_refused() {
    let rendered = errors(
        &info("widgets", Some("acme"), "1.0.0"),
        &with_metamodel(None),
        &MigrateOptions::default(),
    );
    assert!(rendered.contains("declares no `metamodel`"), "{rendered}");
    assert!(rendered.contains("pass `--metamodel <url>`"), "{rendered}");
}

/// A value that is not an IRI is not one of the four either, so it is
/// reported as unrecognized and not with a second message about its
/// syntax.
#[test]
fn a_metamodel_that_is_not_an_iri_is_reported_as_unrecognized() {
    let rendered = errors(
        &info("widgets", Some("acme"), "1.0.0"),
        &with_metamodel(Some("not an iri")),
        &MigrateOptions::default(),
    );
    assert!(rendered.contains("declares `not an iri`"), "{rendered}");
    assert!(!rendered.contains("IRI"), "{rendered}");
}

#[test]
fn the_metamodel_option_supplies_a_missing_one() {
    let validated = migrate_project(
        &info("widgets", Some("acme"), "1.0.0"),
        &with_metamodel(None),
        &supplying(SYSML),
        &no_targets,
    )
    .expect("the option supplies it");
    assert_eq!(declared_metamodel(&validated.value).as_deref(), Some(SYSML));
    // Nothing was discarded, so nothing is said about a metamodel; the
    // `created` warning every migration carries is not this test's.
    assert!(
        !validated
            .warnings
            .iter()
            .any(|w| w.message.contains("metamodel")),
        "{:?}",
        validated.warnings
    );
}

/// Unlike `--publisher`, the option also replaces a declared value --
/// one `model.toml` cannot hold is no more usable than none -- and what
/// was dropped is still reported.
#[test]
fn the_metamodel_option_replaces_an_unrecognized_one() {
    let validated = migrate_project(
        &info("widgets", Some("acme"), "1.0.0"),
        &with_metamodel(Some("https://example.com/spec/SysML/20251201")),
        &supplying(SYSML),
        &no_targets,
    )
    .expect("the option replaces it");
    assert_eq!(declared_metamodel(&validated.value).as_deref(), Some(SYSML));
    assert!(
        validated
            .warnings
            .iter()
            .any(|w| w.message.contains("does not recognize")),
        "{:?}",
        validated.warnings
    );
}

/// model-toml.md gives a recognized value "the same treatment as
/// `name`": it is migrated, not overridden.
#[test]
fn a_declared_metamodel_beats_the_option() {
    let kerml = "https://www.omg.org/spec/KerML/20250201";
    let validated = migrate_project(
        &info("widgets", Some("acme"), "1.0.0"),
        &with_metamodel(Some(kerml)),
        &supplying(SYSML),
        &no_targets,
    )
    .expect("the declared value is used");
    assert_eq!(declared_metamodel(&validated.value).as_deref(), Some(kerml));
}

/// The option is a value the user typed, so it is checked too.
#[test]
fn an_unrecognized_option_is_refused() {
    let rendered = errors(
        &info("widgets", Some("acme"), "1.0.0"),
        &with_metamodel(None),
        &supplying("https://example.com/spec"),
    );
    assert!(
        rendered.contains("`https://example.com/spec` is not a metamodel Sysand recognizes"),
        "{rendered}"
    );
}

// ---------------------------------------------------------- `sources`

fn sources_of(meta: &InterchangeProjectMetadataRaw) -> Vec<String> {
    migrated(&info("widgets", Some("acme"), "1.0.0"), meta)
        .project
        .sources
        .as_ref()
        .expect("sources")
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn sources_come_from_the_index_values() {
    let mut m = meta();
    m.index = IndexMap::from([("A".to_owned(), "a.sysml".to_owned())]);
    assert_eq!(sources_of(&m), vec!["a.sysml".to_owned()]);
}

#[test]
fn sources_come_from_the_checksum_keys() {
    let mut m = meta();
    m.checksum = Some(checksum_of(&["b.kerml"]));
    assert_eq!(sources_of(&m), vec!["b.kerml".to_owned()]);
}

/// The union deduplicates. `index` maps *symbol* to path, so a file
/// exporting three symbols appears three times, and its checksum key a
/// fourth -- which `Sources::new` would refuse outright.
#[test]
fn the_union_is_deduplicated() {
    let mut m = meta();
    m.index = IndexMap::from([
        ("A".to_owned(), "a.sysml".to_owned()),
        ("B".to_owned(), "a.sysml".to_owned()),
        ("C".to_owned(), "b.kerml".to_owned()),
    ]);
    m.checksum = Some(checksum_of(&["a.sysml", "b.kerml"]));
    assert_eq!(
        sources_of(&m),
        vec!["a.sysml".to_owned(), "b.kerml".to_owned()]
    );
}

#[test]
fn no_index_and_no_checksum_is_an_empty_sources() {
    assert_eq!(sources_of(&meta()), [] as [std::string::String; 0]);
}

/// One diagnostic naming *every* offending key.
#[test]
fn every_bad_checksum_key_is_listed_in_one_diagnostic() {
    let mut m = meta();
    m.checksum = Some(checksum_of(&["README.md", "a.sysml", "notes.txt"]));
    let rendered = errors(
        &info("widgets", Some("acme"), "1.0.0"),
        &m,
        &MigrateOptions::default(),
    );
    assert!(rendered.contains("`README.md`"), "{rendered}");
    assert!(rendered.contains("`notes.txt`"), "{rendered}");
    assert!(rendered.contains("run `sysand exclude`"), "{rendered}");
    assert_eq!(
        rendered.matches("run `sysand exclude`").count(),
        1,
        "one diagnostic, not one per key:\n{rendered}"
    );
}

/// An `index` value that is not a model file is an error of its own,
/// which has no plural form.
#[test]
fn a_non_model_index_value_is_an_error() {
    let mut m = meta();
    m.index = IndexMap::from([("A".to_owned(), "README.md".to_owned())]);
    let rendered = errors(
        &info("widgets", Some("acme"), "1.0.0"),
        &m,
        &MigrateOptions::default(),
    );
    assert!(
        rendered.contains("`sources` lists model files only"),
        "{rendered}"
    );
}

// --------------------------------------------------------- `publisher`

#[test]
fn no_publisher_anywhere_is_an_error() {
    let rendered = errors(
        &info("widgets", None, "1.0.0"),
        &meta(),
        &MigrateOptions::default(),
    );
    assert!(
        rendered.contains("declares no `publisher` and none was given"),
        "{rendered}"
    );
}

#[test]
fn the_publisher_comes_from_options_when_the_file_has_none() {
    let manifest = match migrate_project(
        &info("widgets", None, "1.0.0"),
        &meta(),
        &MigrateOptions {
            publisher: Some("acme".to_owned()),
            ..MigrateOptions::default()
        },
        &no_targets,
    )
    .expect("the option supplies it")
    .value
    {
        ModelToml::Project(project) => *project,
        ModelToml::Workspace(_) => unreachable!(),
    };
    assert_eq!(
        manifest.project.publisher.provided().unwrap().as_str(),
        "acme"
    );
}

/// The file's own publisher is migrated rather than overridden. Both
/// values are set, and they differ, or the assertion would pass under
/// either precedence.
#[test]
fn the_files_publisher_wins_over_the_option() {
    let manifest = match migrate_project(
        &info("widgets", Some("fromfile"), "1.0.0"),
        &meta(),
        &MigrateOptions {
            publisher: Some("fromoption".to_owned()),
            ..MigrateOptions::default()
        },
        &no_targets,
    )
    .expect("both are valid")
    .value
    {
        ModelToml::Project(project) => *project,
        ModelToml::Workspace(_) => unreachable!(),
    };
    assert_eq!(
        manifest.project.publisher.provided().unwrap().as_str(),
        "fromfile"
    );
}

// -------------------------------------------------- discarded metadata

#[test]
fn the_discarded_metadata_warns() {
    let mut m = meta();
    m.index = IndexMap::from([("A".to_owned(), "a.sysml".to_owned())]);
    m.checksum = Some(checksum_of(&["a.sysml"]));
    m.includes_derived = Some(true);
    m.includes_implied = Some(false);
    let validated = migrate_project(
        &info("widgets", Some("acme"), "1.0.0"),
        &m,
        &MigrateOptions::default(),
        &no_targets,
    )
    .expect("the conversion succeeds");
    let text = validated
        .warnings
        .iter()
        .map(|w| w.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("`created`"), "{text}");
    assert!(text.contains("`includesDerived`"), "{text}");
    assert!(text.contains("`includesImplied`"), "{text}");
    // The paths are *not* discarded, and the warning has to say so.
    assert!(text.contains("apart from its paths"), "{text}");
}

// ------------------------------------------------------------- usages

fn usage_error(usage: InterchangeProjectUsageRaw) -> String {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = vec![usage];
    errors(&raw, &meta(), &MigrateOptions::default())
}

fn one_usage(usage: InterchangeProjectUsageRaw) -> crate::manifest::usage::Usage {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = vec![usage];
    match migrated(&raw, &meta()).inline_usages.remove(0) {
        crate::manifest::usage::MemberUsage::Declared(usage) => usage,
        crate::manifest::usage::MemberUsage::Inherited {
            publisher: _,
            name: _,
            from: _,
            at: _,
        } => {
            panic!("migrate declares usages, it does not inherit them")
        }
    }
}

#[test]
fn a_directory_usage_converts() {
    let usage = one_usage(InterchangeProjectUsageRaw::Directory {
        dir: "../dep".to_owned(),
        publisher: "acme".to_owned(),
        name: "dep".to_owned(),
    });
    assert!(
        matches!(
            usage,
            crate::manifest::usage::Usage::SourcePath {
                publisher: _,
                name: _,
                dir: _
            }
        ),
        "{usage:?}"
    );
}

#[test]
fn a_kpar_path_usage_converts() {
    let usage = one_usage(InterchangeProjectUsageRaw::KparPath {
        kpar_path: "../dep.kpar".to_owned(),
        publisher: "acme".to_owned(),
        name: "dep".to_owned(),
    });
    assert!(
        matches!(
            usage,
            crate::manifest::usage::Usage::KparPath {
                publisher: _,
                name: _,
                kpar_path: _
            }
        ),
        "{usage:?}"
    );
}

/// model-toml.md "JSON to TOML": a json path is "rewritten into the form a
/// `dir` or `kpar-path` requires", which is the only form a `model.toml`
/// admits.
#[test]
fn a_local_path_is_written_in_normal_form() {
    for (json, written) in [("./dep/", "dep"), ("./../libs/./x/../dep", "../libs/dep")] {
        let usage = one_usage(InterchangeProjectUsageRaw::Directory {
            dir: json.to_owned(),
            publisher: "acme".to_owned(),
            name: "dep".to_owned(),
        });
        assert_eq!(
            usage.local_path().map(|path| path.as_str()),
            Some(written),
            "`{json}`"
        );
    }
    let usage = one_usage(InterchangeProjectUsageRaw::KparPath {
        kpar_path: "./dep.kpar".to_owned(),
        publisher: "acme".to_owned(),
        name: "dep".to_owned(),
    });
    assert_eq!(
        usage.local_path().map(|path| path.as_str()),
        Some("dep.kpar")
    );
}

#[test]
fn a_local_path_that_names_the_project_itself_is_an_error() {
    let rendered = usage_error(InterchangeProjectUsageRaw::Directory {
        dir: "dep/..".to_owned(),
        publisher: "acme".to_owned(),
        name: "dep".to_owned(),
    });
    assert!(
        rendered.contains("usage path `dep/..` names the project itself"),
        "{rendered}"
    );
}

#[test]
fn a_typed_usage_with_an_invalid_publisher_is_an_error() {
    let rendered = usage_error(InterchangeProjectUsageRaw::Directory {
        dir: "../dep".to_owned(),
        publisher: "no".to_owned(),
        name: "dep".to_owned(),
    });
    assert!(rendered.contains("`publisher`"), "{rendered}");
}

#[test]
fn a_constrained_purl_usage_becomes_an_index_usage() {
    let usage = one_usage(InterchangeProjectUsageRaw::Resource {
        resource: "pkg:sysand/acme/dep".to_owned(),
        version_constraint: Some("^1.0.0".to_owned()),
    });
    assert!(
        matches!(
            usage,
            crate::manifest::usage::Usage::Index {
                publisher: _,
                name: _,
                version_constraint: _
            }
        ),
        "{usage:?}"
    );
}

/// An unconstrained purl reaches the lookup, which resolves it: the
/// pinned constraint is the caller's to compute.
#[test]
fn an_unconstrained_purl_usage_reaches_the_lookup() {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = vec![InterchangeProjectUsageRaw::Resource {
        resource: "pkg:sysand/acme/dep".to_owned(),
        version_constraint: None,
    }];
    let lookup = |resource: &str| {
        assert_eq!(resource, "pkg:sysand/acme/dep");
        Ok(TomlUsage::Index {
            publisher: IndexPublisher::parse("acme".to_owned()).expect("a publisher"),
            name: IndexName::parse("dep".to_owned()).expect("a name"),
            version_constraint: semver::VersionReq::parse("^1.2.0").expect("a requirement"),
        })
    };
    let validated = migrate_project(&raw, &meta(), &MigrateOptions::default(), &lookup)
        .expect("the purl resolves");
    let ModelToml::Project(manifest) = validated.value else {
        panic!("a project");
    };
    assert_eq!(manifest.inline_usages.len(), 1);
}

/// An `http(s)` resource reaches the lookup too -- for both kinds of
/// target, which is what model-toml.md's unconditional "use as-is for
/// URL" asks for.
#[test]
fn an_http_usage_reaches_the_lookup() {
    for resource in [
        "https://example.com/dep.kpar",
        "http://example.com/dep/",
        "file:///srv/dep",
    ] {
        let mut raw = info("widgets", Some("acme"), "1.0.0");
        raw.usage = vec![InterchangeProjectUsageRaw::Resource {
            resource: resource.to_owned(),
            version_constraint: None,
        }];
        let lookup = |_: &str| Ok(resolves_to("../dep", "acme", "dep"));
        migrate_project(&raw, &meta(), &MigrateOptions::default(), &lookup)
            .unwrap_or_else(|e| panic!("{resource} resolves: {e}"));
    }
}

#[test]
fn a_git_usage_is_refused() {
    let rendered = usage_error(InterchangeProjectUsageRaw::Resource {
        resource: "git+https://example.com/dep.git".to_owned(),
        version_constraint: None,
    });
    assert!(rendered.contains("git repository"), "{rendered}");
}

// ------------------------------------------------------ all-or-nothing

/// Every problem in one report: a user with four mistakes runs `migrate`
/// once, not four times.
#[test]
fn every_problem_is_reported_together() {
    let mut raw = info("widgets", Some("acme"), "2023-11");
    raw.maintainer = vec!["Ada".to_owned()];
    let mut m = meta();
    m.checksum = Some(checksum_of(&["README.md", "notes.txt"]));

    let rendered = errors(&raw, &m, &MigrateOptions::default());
    assert!(rendered.contains("not a semantic version"), "{rendered}");
    assert!(rendered.contains("maintainer"), "{rendered}");
    assert!(rendered.contains("`README.md`"), "{rendered}");
    assert!(rendered.contains("`notes.txt`"), "{rendered}");
}

// ------------------------------------------------------- `check_vcs`

#[cfg(feature = "networking")]
mod vcs {
    use camino::Utf8Path;
    use camino_tempfile::Utf8TempDir;

    use crate::commands::migrate::{VcsState, check_vcs};

    /// A repository with one committed file.
    ///
    /// `gix` has no staging API, so a clean work tree is built by hand:
    /// blob, tree, commit, then the index materialized from that tree.
    /// `commit_as` rather than `commit`, which needs `user.name` and
    /// `user.email` configured and would break on a machine without a
    /// global git identity.
    fn repository_with(files: &[(&str, &str)]) -> Utf8TempDir {
        let dir = camino_tempfile::tempdir().expect("a temporary directory");
        let repo = gix::init(dir.path()).expect("an empty repository");

        // A git tree holds one path component per entry, so a nested
        // path needs its own subtree object.
        let mut roots: std::collections::BTreeMap<String, Vec<(String, String)>> =
            std::collections::BTreeMap::new();
        for (path, contents) in files {
            let at = dir.path().join(path);
            if let Some(parent) = at.parent() {
                std::fs::create_dir_all(parent).expect("a parent directory");
            }
            std::fs::write(&at, contents).expect("a working-tree file");
            let (dir_part, file_part) = match path.split_once('/') {
                Some((d, f)) => (d.to_owned(), f.to_owned()),
                None => (String::new(), (*path).to_owned()),
            };
            roots
                .entry(dir_part)
                .or_default()
                .push((file_part, (*contents).to_owned()));
        }

        let blob = |contents: &str| repo.write_blob(contents.as_bytes()).expect("a blob").into();
        let mut entries = Vec::new();
        for (dir_part, files) in roots {
            let mut sub: Vec<gix::objs::tree::Entry> = files
                .iter()
                .map(|(name, contents)| gix::objs::tree::Entry {
                    mode: gix::objs::tree::EntryKind::Blob.into(),
                    filename: name.as_str().into(),
                    oid: blob(contents),
                })
                .collect();
            sub.sort();
            if dir_part.is_empty() {
                entries.extend(sub);
            } else {
                let id = repo
                    .write_object(&gix::objs::Tree { entries: sub })
                    .expect("a subtree")
                    .detach();
                entries.push(gix::objs::tree::Entry {
                    mode: gix::objs::tree::EntryKind::Tree.into(),
                    filename: dir_part.as_str().into(),
                    oid: id,
                });
            }
        }
        entries.sort();
        let tree = repo
            .write_object(&gix::objs::Tree { entries })
            .expect("a tree")
            .detach();

        let who = gix::actor::SignatureRef {
            name: "Test".into(),
            email: "test@example.com".into(),
            time: "0 +0000",
        };
        repo.commit_as(
            who,
            who,
            "HEAD",
            "initial",
            tree,
            gix::commit::NO_PARENT_IDS,
        )
        .expect("a commit");

        let tree = repo.find_tree(tree).expect("the tree reads");
        let mut index = repo.index_from_tree(&tree.id()).expect("an index");
        index
            .write(gix::index::write::Options::default())
            .expect("the index is written");
        dir
    }

    #[test]
    fn a_clean_repository_is_clean() {
        let dir = repository_with(&[("a.txt", "one\n")]);
        assert_eq!(check_vcs(dir.path()).expect("a status"), VcsState::Clean);
    }

    #[test]
    fn an_uncommitted_change_is_dirty() {
        let dir = repository_with(&[("a.txt", "one\n")]);
        std::fs::write(dir.path().join("a.txt"), "two\n").expect("a change");
        assert!(
            matches!(check_vcs(dir.path()).expect("a status"), VcsState::Dirty(_)),
            "a modified tracked file is a change"
        );
    }

    #[test]
    fn an_untracked_file_is_dirty() {
        let dir = repository_with(&[("a.txt", "one\n")]);
        std::fs::write(dir.path().join("new.txt"), "new\n").expect("a new file");
        let VcsState::Dirty(paths) = check_vcs(dir.path()).expect("a status") else {
            panic!("an untracked file is a change");
        };
        assert!(paths.iter().any(|p| p == "new.txt"), "{paths:?}");
    }

    #[test]
    fn an_ignored_file_is_not_dirty() {
        let dir = repository_with(&[("a.txt", "one\n"), (".gitignore", "ignored.txt\n")]);
        std::fs::write(dir.path().join("ignored.txt"), "x\n").expect("an ignored file");
        assert_eq!(check_vcs(dir.path()).expect("a status"), VcsState::Clean);
    }

    // `NoRepository` is not tested here. `gix::discover` walks upward,
    // so the test needs `GIT_CEILING_DIRECTORIES` to stop it -- and
    // setting that from a unit test means a process-global `set_var`
    // that races every other test in the binary. A CLI test can set it
    // on the child process, where it is per-process and safe.

    /// `discover`, not `open`: a project in a subdirectory of a
    /// repository is inside its work tree.
    #[test]
    fn a_subdirectory_of_a_repository_is_inside_it() {
        let dir = repository_with(&[("a.txt", "one\n")]);
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).expect("a subdirectory");
        assert_eq!(check_vcs(&sub).expect("a status"), VcsState::Clean);
    }

    /// The status is scoped to the project: a change elsewhere in a
    /// monorepo must not refuse a migration.
    #[test]
    fn a_change_outside_the_project_does_not_make_it_dirty() {
        let dir = repository_with(&[("app/a.txt", "one\n"), ("other/b.txt", "two\n")]);
        std::fs::write(dir.path().join("other/b.txt"), "changed\n").expect("a change");

        let app: &Utf8Path = &dir.path().join("app");
        assert_eq!(check_vcs(app).expect("a status"), VcsState::Clean);
        assert!(
            matches!(check_vcs(dir.path()).expect("a status"), VcsState::Dirty(_)),
            "the change is still a change at the root"
        );
    }
}

// ---------------------------------------------------- duplicate usages

fn dir_usage(dir: &str, publisher: &str, name: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Directory {
        dir: dir.to_owned(),
        publisher: publisher.to_owned(),
        name: name.to_owned(),
    }
}

fn usage_errors(usages: Vec<InterchangeProjectUsageRaw>) -> String {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = usages;
    errors(&raw, &meta(), &MigrateOptions::default())
}

/// Without this, `migrate` writes a manifest `sysand` cannot read --
/// after deleting the json pair that was the only readable copy.
#[test]
fn two_usages_of_one_identity_are_refused() {
    let rendered = usage_errors(vec![
        dir_usage("../dep", "acme", "dep"),
        dir_usage("../other", "acme", "dep"),
    ]);
    assert!(rendered.contains("name the same project"), "{rendered}");
}

/// The identities are normalized, so `Acme/Dep` collides with
/// `acme/dep`. Literal comparison lets this through.
#[test]
fn a_duplicate_is_normalized_not_literal() {
    let rendered = usage_errors(vec![
        dir_usage("../dep", "acme", "dep"),
        dir_usage("../other", "Acme", "Dep"),
    ]);
    assert!(rendered.contains("name the same project"), "{rendered}");
}

/// Pairwise, as the read path is: three colliding entries give two
/// diagnostics, not one and not three.
#[test]
fn three_colliding_usages_report_twice() {
    let rendered = usage_errors(vec![
        dir_usage("../a", "acme", "dep"),
        dir_usage("../b", "acme", "dep"),
        dir_usage("../c", "acme", "dep"),
    ]);
    assert_eq!(
        rendered.matches("name the same project").count(),
        2,
        "{rendered}"
    );
}

/// Two usages of *different* projects are not a duplicate.
#[test]
fn two_distinct_usages_are_kept() {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = vec![
        dir_usage("../alpha", "acme", "alpha"),
        dir_usage("../beta", "acme", "beta"),
    ];
    assert_eq!(migrated(&raw, &meta()).inline_usages.len(), 2);
}

// --------------------------------------------------- `file:` resources

use crate::manifest::usage::Usage as TomlUsage;
use crate::model::{IndexName, IndexPublisher};

fn file_resource(resource: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Resource {
        resource: resource.to_owned(),
        version_constraint: None,
    }
}

/// A lookup that answers with a fixed usage, standing in for one that
/// reads the target. The core stays fixtureless: no directory is read
/// here.
fn resolves_to(dir: &str, publisher: &str, name: &str) -> TomlUsage {
    TomlUsage::SourcePath {
        publisher: IndexPublisher::parse(publisher.to_owned()).expect("a publisher"),
        name: IndexName::parse(name.to_owned()).expect("a name"),
        dir: typed_path::Utf8UnixPathBuf::from(dir),
    }
}

#[test]
fn a_file_usage_resolves_from_the_target() {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = vec![file_resource("file:///srv/dep")];
    let lookup = |_: &str| Ok(resolves_to("../dep", "acme", "dep"));

    let validated = migrate_project(&raw, &meta(), &MigrateOptions::default(), &lookup)
        .expect("the target resolves");
    let ModelToml::Project(manifest) = validated.value else {
        panic!("a project");
    };
    assert_eq!(manifest.inline_usages.len(), 1, "the usage is kept");
}

#[test]
fn a_file_usage_whose_target_cannot_be_read_is_refused() {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = vec![file_resource("file:///srv/dep")];
    let lookup = |_: &str| Err("`/srv/dep` declares no `publisher`".to_owned());

    let rendered = migrate_project(&raw, &meta(), &MigrateOptions::default(), &lookup)
        .expect_err("the target cannot be read")
        .to_string();
    assert!(rendered.contains("declares no `publisher`"), "{rendered}");
}

/// Two `file:` usages naming one target are a duplicate.
#[test]
fn two_file_usages_of_one_target_are_a_duplicate() {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = vec![
        file_resource("file:///srv/dep"),
        file_resource("file:///srv/other"),
    ];
    let lookup = |_: &str| Ok(resolves_to("../dep", "acme", "dep"));

    let rendered = migrate_project(&raw, &meta(), &MigrateOptions::default(), &lookup)
        .expect_err("both name `acme/dep`")
        .to_string();
    assert!(rendered.contains("name the same project"), "{rendered}");
}

/// model-toml.md: an `http(s)` usage's `versionConstraint` is discarded
/// with a warning. The warning belongs in core: the target lookup is
/// handed only the resource string and never sees the constraint.
#[test]
fn an_http_usage_discards_its_constraint_with_a_warning() {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = vec![InterchangeProjectUsageRaw::Resource {
        resource: "https://example.com/dep.kpar".to_owned(),
        version_constraint: Some("^1.0.0".to_owned()),
    }];
    let lookup = |_: &str| Ok(resolves_to("../dep", "acme", "dep"));
    let validated = migrate_project(&raw, &meta(), &MigrateOptions::default(), &lookup)
        .expect("the usage resolves; the constraint is only discarded");
    let text = validated
        .warnings
        .iter()
        .map(|w| w.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("a `kpar-url` usage carries no version constraint"),
        "{text}"
    );
}

// ------------------------------------ the language the metamodel admits

/// `checksum` keys that *are* model files, of the language this
/// project's metamodel does not admit. A second message,
/// not a second kind of entry in the first: "not a KerML or SysML file"
/// has no metamodel to name.
#[test]
fn a_checksum_key_the_metamodel_does_not_admit_names_the_metamodel() {
    let mut m = with_metamodel(Some("https://www.omg.org/spec/KerML/20250201"));
    m.checksum = Some(checksum_of(&["a.kerml", "b.sysml", "c.sysml"]));
    let rendered = errors(
        &info("widgets", Some("acme"), "1.0.0"),
        &m,
        &MigrateOptions::default(),
    );
    assert!(rendered.contains("`b.sysml`, `c.sysml`"), "{rendered}");
    assert!(
        rendered.contains(
            "whose type the metamodel `https://www.omg.org/spec/KerML/20250201` \
                           does not admit"
        ),
        "{rendered}"
    );
    assert!(!rendered.contains("`a.kerml`"), "{rendered}");
    assert_eq!(
        rendered.matches("run `sysand exclude`").count(),
        1,
        "one diagnostic for the set:\n{rendered}"
    );
}

/// An `index` value of the wrong language is reported per entry, as an
/// `index` value that is no model file is.
#[test]
fn an_index_value_the_metamodel_does_not_admit_is_reported_per_entry() {
    let mut m = with_metamodel(Some("https://www.omg.org/spec/KerML/20250201"));
    m.index = IndexMap::from([("Sym".to_owned(), "b.sysml".to_owned())]);
    let rendered = errors(
        &info("widgets", Some("acme"), "1.0.0"),
        &m,
        &MigrateOptions::default(),
    );
    assert!(
        rendered.contains(
            "`sources` entry `b.sysml` is a SysML file, which the metamodel \
             `https://www.omg.org/spec/KerML/20250201` does not admit"
        ),
        "{rendered}"
    );
}

/// The checks are skipped when the metamodel did not resolve: that has
/// been reported already, and a second complaint about a language nobody knows
/// would only crowd the report.
#[test]
fn an_unresolved_metamodel_skips_the_language_check() {
    let mut m = with_metamodel(None);
    m.checksum = Some(checksum_of(&["b.sysml"]));
    let rendered = errors(
        &info("widgets", Some("acme"), "1.0.0"),
        &m,
        &MigrateOptions::default(),
    );
    assert!(rendered.contains("declares no `metamodel`"), "{rendered}");
    assert!(!rendered.contains("does not admit"), "{rendered}");
}

/// A json usage's spellings only had to match after normalization, so a
/// migrated manifest has to take the target's. The CLI does the
/// rewriting -- it has the filesystem -- and this pins what it is for:
/// without it `migrate` writes a `model.toml` that `sysand lock`
/// refuses, after deleting the json pair it was converted from.
#[test]
fn a_typed_usage_keeps_the_json_spelling_here() {
    let mut raw = info("widgets", Some("acme"), "1.0.0");
    raw.usage = vec![InterchangeProjectUsageRaw::Directory {
        dir: "../dep".to_owned(),
        publisher: "acme-corp".to_owned(),
        name: "dep".to_owned(),
    }];
    let manifest = migrated(&raw, &meta());
    let usage = &manifest.inline_usages[0];
    let crate::manifest::usage::MemberUsage::Declared(usage) = usage else {
        panic!("a declared usage");
    };
    assert_eq!(usage.publisher().as_str(), "acme-corp");
}
