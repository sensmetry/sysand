// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use super::*;

use crate::manifest::usage::LocatedUsage;

fn publisher(raw: &str) -> IndexPublisher {
    IndexPublisher::parse(raw).expect("a valid publisher")
}

fn name(raw: &str) -> IndexName {
    IndexName::parse(raw).expect("a valid name")
}

fn index(raw_publisher: &str, raw_name: &str, constraint: &str) -> Usage {
    Usage::Index {
        publisher: publisher(raw_publisher),
        name: name(raw_name),
        version_constraint: semver::VersionReq::parse(constraint).expect("a constraint"),
    }
}

fn dir(raw_publisher: &str, raw_name: &str, path: &str) -> Usage {
    Usage::SourcePath {
        publisher: publisher(raw_publisher),
        name: name(raw_name),
        dir: Utf8UnixPathBuf::from(path),
    }
}

fn kpar_path(raw_publisher: &str, raw_name: &str, path: &str) -> Usage {
    Usage::KparPath {
        publisher: publisher(raw_publisher),
        name: name(raw_name),
        kpar_path: Utf8UnixPathBuf::from(path),
    }
}

fn url(raw_publisher: &str, raw_name: &str, iri: &str) -> Usage {
    Usage::RemoteKpar {
        publisher: Some(publisher(raw_publisher)),
        name: name(raw_name),
        url: Iri::parse(iri.to_owned()).expect("a valid IRI"),
    }
}

/// model-toml.md "TOML to KPAR": "index: turn into a PURL
/// (`pkg:sysand/<publisher>/<name>`) with the version constraint as
/// declared".
#[test]
fn an_index_usage_becomes_a_purl_resource() {
    assert_eq!(
        usage(&index("acme", "widgets", "^1.2.3")),
        InterchangeProjectUsageRaw::Resource {
            resource: "pkg:sysand/acme/widgets".to_owned(),
            version_constraint: Some("^1.2.3".to_owned()),
        }
    );
}

/// A PURL is built from the *normalized* pair: model-toml.md
/// "Normalization", "All projects are resolved by their normalized
/// identity, regardless of how the usage is declared."
#[test]
fn a_purl_resource_is_built_from_the_normalized_pair() {
    assert_eq!(
        usage(&index("Acme Corp", "Wid Get", "^1")),
        InterchangeProjectUsageRaw::Resource {
            resource: "pkg:sysand/acme-corp/wid-get".to_owned(),
            version_constraint: Some("^1".to_owned()),
        }
    );
}

/// model-toml.md "TOML to KPAR": "remote-kpar: use the URL as-is"; the
/// kind takes no version constraint (Usages).
#[test]
fn a_kpar_url_usage_becomes_a_resource_with_no_constraint() {
    let target = "https://example.com/a/b.kpar?v=2#frag";
    assert_eq!(
        usage(&url("acme", "widgets", target)),
        InterchangeProjectUsageRaw::Resource {
            resource: target.to_owned(),
            version_constraint: None,
        }
    );
}

/// The view keeps a local usage typed, where the archive would rewrite
/// it into a PURL constrained to the target's version -- which needs the
/// target, and the resolver needs the path.
#[test]
fn a_dir_usage_becomes_a_typed_directory_usage() {
    assert_eq!(
        usage(&dir("acme", "widgets", "../lib")),
        InterchangeProjectUsageRaw::Directory {
            dir: "../lib".to_owned(),
            publisher: "acme".to_owned(),
            name: "widgets".to_owned(),
        }
    );
}

#[test]
fn a_kpar_path_usage_becomes_a_typed_kpar_usage() {
    assert_eq!(
        usage(&kpar_path("acme", "widgets", "vendor/lib.kpar")),
        InterchangeProjectUsageRaw::KparPath {
            kpar_path: "vendor/lib.kpar".to_owned(),
            publisher: "acme".to_owned(),
            name: "widgets".to_owned(),
        }
    );
}

/// model-toml.md "TOML to KPAR": "A typed usage carries `publisher` and
/// `name` exactly as the manifest spells them, without normalization,
/// because Sysand requires them to match the values found at the
/// target." The opposite of the PURL above.
#[test]
fn a_typed_usage_keeps_the_pair_unnormalized() {
    assert_eq!(
        usage(&dir("Acme Corp", "Wid Get", "../lib")),
        InterchangeProjectUsageRaw::Directory {
            dir: "../lib".to_owned(),
            publisher: "Acme Corp".to_owned(),
            name: "Wid Get".to_owned(),
        }
    );
}

/// A view is consumed by the same code that reads a real KPAR, so every
/// usage it writes has to be one that code accepts. For the index kind
/// this is what pins the normalization above: `validate` refuses a
/// `pkg:sysand` IRI that is not in normalized form.
#[test]
fn a_view_usage_is_one_sysand_accepts() {
    for declared in [
        index("Acme Corp", "Wid Get", "^1.2.3"),
        url("acme", "widgets", "https://example.com/a/b.kpar"),
        dir("acme", "widgets", "../lib"),
        kpar_path("acme", "widgets", "vendor/lib.kpar"),
    ] {
        usage(&declared)
            .validate()
            .unwrap_or_else(|e| panic!("{declared:?} does not validate: {e}"));
    }
}

/// The identity of a usage is guaranteed encodable, but its *path* is
/// not checked at all when the manifest is read, unlike `sources`. So a
/// manifest the reader accepts can still produce a view that no consumer
/// validates -- with a message that names neither the key nor the file.
///
/// Pinned rather than fixed: the fix belongs to the manifest reader,
/// which would apply `parse_relative_unix_path` with the same two path
/// kinds `validate` uses here.
#[test]
fn a_usage_path_the_manifest_accepts_may_still_not_validate() {
    let error = usage(&dir("acme", "widgets", ""))
        .validate()
        .expect_err("an empty path");
    assert!(
        error.to_string().contains("references an invalid path"),
        "unexpected message: {error}"
    );
}

// ----- the pair -----

use std::collections::{BTreeMap, HashSet};

use crate::manifest::project::ProjectManifest;
use crate::manifest::spans::Positions;
use crate::manifest::spans::Source;

/// A `[project]` whose body is `body`, read and resolved outside any
/// workspace -- which is what a standalone `model.toml` project is.
fn resolved(body: &str) -> ResolvedProject {
    let text = format!("[project]\n{body}");
    let (table, positions) = Source::new("model.toml", &text)
        .read()
        .expect("the test document parses");
    ProjectManifest::validate(&table, &positions)
        .expect("the manifest validates")
        .into_parts()
        .0
        .resolve(None)
        .expect("the manifest resolves")
        .into_parts()
        .0
}

/// The keys `[project]` requires: the three every metamodel needs, the
/// `metamodel` itself, and the `sources` an OMG-metamodel project needs.
const REQUIRED: &str = "\
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
";

/// A reader over a fixed set of files, standing in for a directory.
fn files(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(path, contents)| ((*path).to_owned(), (*contents).to_owned()))
        .collect()
}

fn reader(files: &BTreeMap<String, String>) -> impl FnMut(&Utf8UnixPath) -> Result<String, String> {
    move |path| {
        files
            .get(path.as_str())
            .cloned()
            .ok_or_else(|| format!("no such file `{path}`"))
    }
}

fn maps(body: &str, entries: &[(&str, &str)]) -> (SymbolIndex, ChecksumMap) {
    let project = resolved(body);
    let files = files(entries);
    source_maps(&project.sources, reader(&files)).expect("the sources are readable")
}

const FULL: &str = "\
publisher = \"Acme Corp\"
name = \"widgets\"
version = \"1.2.3\"
sources = [\"a.sysml\"]
description = \"A description.\"
license = \"MIT OR Apache-2.0\"
topics = [\"systems\", \"demo\"]
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
maintainers = [\"Fine Person <fine@example.com>\"]

[project.urls]
website = \"https://example.com/\"
documentation = \"https://example.com/docs\"
repository = \"https://example.com/git\"
";

#[test]
fn the_project_table_maps_onto_project_json() {
    let info = info(&resolved(FULL));
    assert_eq!(info.name, "widgets");
    assert_eq!(info.publisher.as_deref(), Some("Acme Corp"));
    assert_eq!(info.version, "1.2.3");
    assert_eq!(info.description.as_deref(), Some("A description."));
    // `spdx::Expression` prints the string it was parsed from, so the
    // manifest's spelling survives.
    assert_eq!(info.license.as_deref(), Some("MIT OR Apache-2.0"));
    assert_eq!(info.maintainer, ["Fine Person <fine@example.com>"]);
    assert_eq!(info.website.as_deref(), Some("https://example.com/"));
    assert_eq!(info.topic, ["systems", "demo"]);
    assert_eq!(
        info.usage,
        [] as [crate::model::InterchangeProjectUsageG<
            std::string::String,
            std::string::String,
            std::string::String,
            std::string::String,
            std::string::String,
            std::string::String,
            std::string::String,
        >; 0]
    );
}

#[test]
fn a_minimal_project_writes_only_what_it_has() {
    let info = info(&resolved(REQUIRED));
    assert_eq!(info.publisher.as_deref(), Some("acme"));
    assert_eq!(info.description, None);
    assert_eq!(info.license, None);
    assert_eq!(info.website, None);
    assert_eq!(info.maintainer, [] as [std::string::String; 0]);
    assert_eq!(info.topic, [] as [std::string::String; 0]);
    assert_eq!(
        info.usage,
        [] as [crate::model::InterchangeProjectUsageG<
            std::string::String,
            std::string::String,
            std::string::String,
            std::string::String,
            std::string::String,
            std::string::String,
            std::string::String,
        >; 0]
    );
}

/// model-toml.md "Standard library" omits `publisher` from the
/// `.project.json` of a project carrying `stdlib` -- from the *archive*.
/// The view keeps it, or the standard library's own `dir` usages, which
/// require the target to declare the publisher they name, could never
/// resolve.
#[test]
fn a_stdlib_project_keeps_its_publisher_in_the_view() {
    let info = info(&resolved(
        "publisher = \"OMG\"\n\
         name = \"Kernel Semantic Library\"\n\
         version = \"1.0.0\"\n\
         sources = []\n\
         stdlib = \"Semantic-Library\"\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n",
    ));
    assert_eq!(info.publisher.as_deref(), Some("OMG"));
}

/// "`urls.repository` and `urls.documentation` have no counterpart in
/// `.project.json` and are dropped."
#[test]
fn documentation_and_repository_urls_are_dropped() {
    let info = info(&resolved(FULL));
    assert_eq!(info.website.as_deref(), Some("https://example.com/"));
    let rendered = serde_json::to_string(&info).expect("serializes");
    assert!(!rendered.contains("example.com/docs"), "{rendered}");
    assert!(!rendered.contains("example.com/git"), "{rendered}");
}

#[test]
fn usages_are_mapped_in_manifest_order() {
    let info = info(&resolved(&format!(
        "{REQUIRED}\n\
         [[inline-usages]]\n\
         publisher = \"acme\"\n\
         name = \"first\"\n\
         version = \"^1\"\n\n\
         [[inline-usages]]\n\
         publisher = \"acme\"\n\
         name = \"second\"\n\
         dir = \"../second\"\n\n\
         [[inline-usages]]\n\
         publisher = \"acme\"\n\
         name = \"third\"\n\
         kpar-url = \"https://example.com/third.kpar\"\n"
    )));
    assert_eq!(
        info.usage,
        [
            InterchangeProjectUsageRaw::Resource {
                resource: "pkg:sysand/acme/first".to_owned(),
                version_constraint: Some("^1".to_owned()),
            },
            InterchangeProjectUsageRaw::Directory {
                dir: "../second".to_owned(),
                publisher: "acme".to_owned(),
                name: "second".to_owned(),
            },
            InterchangeProjectUsageRaw::Resource {
                resource: "https://example.com/third.kpar".to_owned(),
                version_constraint: None,
            },
        ]
    );
}

/// A `LocatedUsage` also records which directory its paths are relative
/// to -- the project's own for a declared usage, the workspace root for
/// an inherited one -- and a `.project.json` has no way to say which. So
/// the two are indistinguishable in a view, and anything that needs the
/// base has to ask the manifest instead.
#[test]
fn an_inherited_usage_is_written_like_a_declared_one() {
    let mut project = resolved(REQUIRED);
    let declared = dir("acme", "lib", "../lib");
    project.usages = vec![
        LocatedUsage::declared(declared.clone()),
        LocatedUsage::inherited(declared),
    ];
    let written = info(&project).usage;
    assert_eq!(written.len(), 2);
    assert_eq!(written[0], written[1]);
}

#[test]
fn project_json_from_the_view_validates() {
    let project = resolved(FULL);
    info(&project).validate().expect("a valid `.project.json`");
}

#[test]
fn the_metamodel_is_copied_into_meta_json() {
    let project = resolved(FULL);
    let meta = meta(
        &project,
        &jiff::Timestamp::UNIX_EPOCH,
        SymbolIndex::new(),
        ChecksumMap::new(),
    );
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.omg.org/spec/SysML/20250201")
    );
}

/// `created` is written the one way `InterchangeProjectMetadataRaw` is
/// documented to be written, so that two documents recording the same
/// instant are byte-identical.
#[test]
fn created_is_formatted_as_the_metadata_helper_formats_it() {
    let instant: jiff::Timestamp = "2026-09-14T12:34:56Z".parse().expect("an instant");
    let project = resolved(REQUIRED);
    let meta = meta(&project, &instant, SymbolIndex::new(), ChecksumMap::new());
    assert_eq!(meta.created, crate::model::format_created(&instant));
}

#[test]
fn the_derived_and_implied_flags_are_omitted() {
    let project = resolved(REQUIRED);
    let meta = meta(
        &project,
        &jiff::Timestamp::UNIX_EPOCH,
        SymbolIndex::new(),
        ChecksumMap::new(),
    );
    assert_eq!(meta.includes_derived, None);
    assert_eq!(meta.includes_implied, None);
}

#[test]
fn every_source_gets_a_checksum_entry() {
    let (_, checksum) = maps(
        &REQUIRED.replace(
            "sources = []",
            "sources = [\"b.sysml\", \"a.kerml\", \"c.sysml\"]",
        ),
        &[
            ("b.sysml", "part def B;"),
            ("a.kerml", "package A;"),
            ("c.sysml", "part def C;"),
        ],
    );
    assert_eq!(
        checksum.keys().collect::<Vec<_>>(),
        ["b.sysml", "a.kerml", "c.sysml"]
    );
}

#[test]
fn a_checksum_is_the_sha256_of_the_contents() {
    let (_, checksum) = maps(
        &REQUIRED.replace("sources = []", "sources = [\"a.sysml\"]"),
        &[("a.sysml", "part def A;")],
    );
    let entry = &checksum["a.sysml"];
    assert_eq!(
        entry.value,
        crate::utils::sha256_lowercase_hex("part def A;")
    );
    assert_eq!(entry.algorithm, "SHA256");
}

#[test]
fn every_top_level_symbol_points_at_its_file() {
    let (index, _) = maps(
        &REQUIRED.replace("sources = []", "sources = [\"a.sysml\", \"b.kerml\"]"),
        &[
            ("a.sysml", "part def A;\npart def B;"),
            ("b.kerml", "package C;"),
        ],
    );
    assert_eq!(
        index.iter().collect::<Vec<_>>(),
        [
            (&"A".to_owned(), &"a.sysml".to_owned()),
            (&"B".to_owned(), &"a.sysml".to_owned()),
            (&"C".to_owned(), &"b.kerml".to_owned()),
        ]
    );
}

/// `index` maps a symbol onto one path, so a symbol declared twice keeps
/// the later file.
#[test]
fn a_symbol_in_two_files_points_at_the_last() {
    let (index, _) = maps(
        &REQUIRED.replace("sources = []", "sources = [\"a.sysml\", \"b.sysml\"]"),
        &[("a.sysml", "part def A;"), ("b.sysml", "part def A;")],
    );
    assert_eq!(index["A"], "b.sysml");
}

#[test]
fn a_source_with_no_symbols_still_gets_a_checksum() {
    let (index, checksum) = maps(
        &REQUIRED.replace("sources = []", "sources = [\"empty.sysml\"]"),
        &[("empty.sysml", "")],
    );
    assert!(index.is_empty());
    assert!(checksum.contains_key("empty.sysml"));
}

/// model-toml.md's reason for not writing `sources` into a KPAR: "the
/// paths it lists are recoverable from these two maps, which is exactly
/// the union the reverse direction takes".
#[test]
fn the_two_maps_recover_the_source_list() {
    let body = REQUIRED.replace("sources = []", "sources = [\"a.sysml\", \"empty.sysml\"]");
    let project = resolved(&body);
    let files = files(&[("a.sysml", "part def A;"), ("empty.sysml", "")]);
    let (index, checksum) = source_maps(&project.sources, reader(&files)).expect("readable");
    let meta = meta(
        &project,
        &jiff::Timestamp::UNIX_EPOCH,
        index.clone(),
        checksum,
    );

    let sources: HashSet<String> = project
        .sources
        .iter()
        .map(|path| path.as_str().to_owned())
        .collect();
    assert_eq!(meta.source_paths(true), sources);
    // The index never points outside the list, so the union does not
    // depend on which half it is taken from.
    assert_eq!(meta.source_paths(false), meta.source_paths(true));
    assert!(index.values().all(|path| sources.contains(path)));
}

#[test]
fn a_project_with_no_sources_gets_two_empty_maps() {
    let (index, checksum) = maps(REQUIRED, &[]);
    assert!(index.is_empty());
    assert!(checksum.is_empty());
}

#[test]
fn meta_json_from_the_view_validates() {
    let project = resolved(FULL);
    let files = files(&[("a.sysml", "part def A;")]);
    let (_, meta) = view(&project, &jiff::Timestamp::now(), reader(&files)).expect("readable");
    // The strict validator -- not the partial one, which accepts a
    // `metamodel` that is no URL and a `created` that is no timestamp.
    meta.validate().expect("a valid `.meta.json`");
}

/// The guard on the test above: a `metamodel` outside the four cannot
/// reach a view, because the manifest carrying one does not read. Were
/// that to change, `meta_json_from_the_view_validates` would start
/// passing for the wrong reason.
#[test]
fn an_unrecognized_metamodel_never_reaches_the_view() {
    let text = format!(
        "[project]\n{}metamodel = \"not a url at all\"\n",
        REQUIRED.replace(
            "metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
            ""
        )
    );
    let (table, positions) = Source::new("model.toml", &text)
        .read()
        .expect("the document parses");
    ProjectManifest::validate(&table, &positions).expect_err("the manifest is rejected");
}

#[test]
fn a_source_that_cannot_be_read_reports_the_readers_error() {
    let project = resolved(&REQUIRED.replace("sources = []", "sources = [\"a.sysml\"]"));
    let files = files(&[]);
    let error = source_maps(&project.sources, reader(&files)).expect_err("nothing to read");
    assert_eq!(error.to_string(), "no such file `a.sysml`");
}

#[test]
fn a_source_that_does_not_parse_stops_the_view() {
    let project = resolved(&REQUIRED.replace("sources = []", "sources = [\"a.sysml\"]"));
    let files = files(&[("a.sysml", "package Foo { /*")]);
    let error = source_maps(&project.sources, reader(&files)).expect_err("an unterminated comment");
    assert!(
        error
            .to_string()
            .starts_with("failed to extract symbol names from `a.sysml`: "),
        "unexpected message: {error}"
    );
}

/// Unlike a manifest report, a view does not accumulate: the caller's
/// error is one value, and a build that cannot read one of its sources
/// has nothing to go on.
#[test]
fn reading_stops_at_the_first_failure() {
    let project =
        resolved(&REQUIRED.replace("sources = []", "sources = [\"a.sysml\", \"b.sysml\"]"));
    let files = files(&[]);
    let error = source_maps(&project.sources, reader(&files)).expect_err("nothing to read");
    assert_eq!(error.to_string(), "no such file `a.sysml`");
}

/// The entry point is the two halves and cannot drift from them.
#[test]
fn the_view_is_the_two_halves() {
    let project = resolved(FULL);
    let files = files(&[("a.sysml", "part def A;")]);
    let created = jiff::Timestamp::UNIX_EPOCH;

    let (whole_info, whole_meta) = view(&project, &created, reader(&files)).expect("readable");
    let (index, checksum) = source_maps(&project.sources, reader(&files)).expect("readable");
    assert_eq!(whole_info, info(&project));
    assert_eq!(whole_meta, meta(&project, &created, index, checksum));
}

// ----- the view of a manifest text -----

const MANIFEST: &str = "\
manifest-version = \"1.0.0\"

[project]
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
";

fn nothing_to_read(path: &Utf8UnixPath) -> Result<String, String> {
    Err(format!("no such file `{path}`"))
}

#[test]
fn a_manifest_that_does_not_parse_is_reported_as_a_deserialization_error() {
    let error =
        of_text("model.toml", "this is not toml\n", nothing_to_read).expect_err("not a manifest");
    assert!(
        matches!(error, ViewError::Manifest(_)),
        "unexpected error: {error}"
    );
    assert!(
        error.to_string().contains("`model.toml` is not valid TOML"),
        "unexpected message: {error}"
    );
}

/// A view's `created` is not the instant it was taken, because the
/// project checksum covers it and that checksum reaches the lockfile.
#[test]
fn the_view_of_a_text_records_the_fixed_created() {
    let (_, meta) = of_text("model.toml", MANIFEST, nothing_to_read).expect("no sources to read");
    assert_eq!(meta.created, crate::model::format_created(&VIEW_CREATED));
    assert_eq!(meta.created, "1970-01-01T00:00:00Z");
}

// --- reading a json usage as a manifest usage -------------------------

use super::{NoManifestForm, usage_of_raw};
use crate::model::{IndexName, IndexPublisher};

fn raw_resource(resource: &str, constraint: Option<&str>) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Resource {
        resource: resource.to_owned(),
        version_constraint: constraint.map(str::to_owned),
    }
}

#[test]
fn a_sysand_purl_with_a_constraint_is_an_index_usage() {
    let usage = usage_of_raw(
        &raw_resource("pkg:sysand/acme/lib", Some("^1.2")),
        ".project.json",
    )
    .expect("an index usage");

    assert_eq!(
        usage,
        Usage::Index {
            publisher: crate::model::IndexPublisher::parse("acme").expect("a publisher"),
            name: crate::model::IndexName::parse("lib").expect("a name"),
            version_constraint: semver::VersionReq::parse("^1.2").expect("a constraint"),
        }
    );
}

#[test]
fn an_index_usage_needs_a_constraint() {
    // `Usage::Index` carries a `VersionReq` and the reader refuses an
    // entry without one. The specification resolves this from the index
    // at migration time; this refusal asks for it instead.
    let error = usage_of_raw(&raw_resource("pkg:sysand/acme/lib", None), ".project.json")
        .expect_err("no constraint");

    assert!(matches!(
        error,
        NoManifestForm::NoConstraint {
            publisher: _,
            name: _
        }
    ));
    assert!(
        error
            .to_string()
            .contains("sysand add acme/lib <constraint>")
    );
}

#[test]
fn an_iri_that_is_not_a_sysand_purl_has_no_manifest_form() {
    let error = usage_of_raw(&raw_resource("urn:kpar:something", None), ".project.json")
        .expect_err("untyped");

    assert!(matches!(error, NoManifestForm::UntypedIri(_)));
    assert!(
        error
            .to_string()
            .contains("named by its publisher and name")
    );
}

#[test]
fn a_directory_usage_becomes_a_dir_entry() {
    let usage = usage_of_raw(
        &InterchangeProjectUsageRaw::Directory {
            dir: "../lib".to_owned(),
            publisher: "acme".to_owned(),
            name: "lib".to_owned(),
        },
        ".project.json",
    )
    .expect("a dir usage");

    assert!(matches!(
        usage,
        Usage::SourcePath {
            publisher: _,
            name: _,
            dir: _
        }
    ));
}

#[test]
fn a_kpar_path_usage_becomes_a_kpar_path_entry() {
    let usage = usage_of_raw(
        &InterchangeProjectUsageRaw::KparPath {
            kpar_path: "vendor/lib.kpar".to_owned(),
            publisher: "acme".to_owned(),
            name: "lib".to_owned(),
        },
        ".project.json",
    )
    .expect("a kpar-path usage");

    assert!(matches!(
        usage,
        Usage::KparPath {
            publisher: _,
            name: _,
            kpar_path: _
        }
    ));
}

#[test]
fn a_target_whose_identifiers_the_rule_refuses_names_its_file() {
    // A raw typed usage's publisher and name are unvalidated strings
    // copied from the target's `.project.json`. The fix is in that
    // file, so the message says which.
    let error = usage_of_raw(
        &InterchangeProjectUsageRaw::Directory {
            dir: "../lib".to_owned(),
            publisher: "x_y".to_owned(),
            name: "lib".to_owned(),
        },
        "../lib/.project.json",
    )
    .expect_err("an identifier a manifest cannot hold");

    assert!(matches!(
        error,
        NoManifestForm::Identifier {
            file: _,
            problem: _
        }
    ));
    assert!(error.to_string().contains("../lib/.project.json"));
}

#[test]
fn the_forward_map_normalizes_and_this_one_does_not() {
    // Not an inverse: `usage` writes the normalized PURL, so a
    // publisher written `ACME` comes back `acme`.
    let shouted = Usage::Index {
        publisher: crate::model::IndexPublisher::parse("ACME").expect("a publisher"),
        name: crate::model::IndexName::parse("Lib").expect("a name"),
        version_constraint: semver::VersionReq::parse("^1").expect("a constraint"),
    };

    let back = usage_of_raw(&super::usage(&shouted), ".project.json").expect("an index usage");
    assert_eq!(back.publisher().expect("a publisher").as_str(), "acme");
    assert_eq!(back.name().as_str(), "lib");
}

// ------------------- resolving against a workspace table

const INHERITS_VERSION: &str = "manifest-version = \"1.0.0\"\n\n\
     [project]\n\
     publisher = \"acme\"\n\
     name = \"widgets\"\n\
     version.workspace = true\n\
     sources = []\n\
     metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n";

#[test]
fn resolved_of_text_in_follows_the_table() {
    let table = crate::manifest::workspace_table::WorkspaceTable {
        default_group: crate::manifest::workspace_table::GroupProperties {
            version: Some("9.9.9".parse().expect("a version")),
            ..Default::default()
        },
        ..Default::default()
    };
    let project = super::resolved_of_text_in(MODEL_TOML_FILE, INHERITS_VERSION, Some(&table))
        .expect("the marker follows the table");
    assert_eq!(project.version.to_string(), "9.9.9");
}

/// The wrapper without a table refuses, and the *message* says why: a
/// default table would not refuse, it would say the group does not give
/// the key, so `is_err` alone would not tell the two apart.
#[test]
fn resolved_of_text_still_refuses_without_a_table() {
    let error = super::resolved_of_text(MODEL_TOML_FILE, INHERITS_VERSION)
        .expect_err("no table, no inheritance");
    assert!(
        error.to_string().contains("not a member of any workspace"),
        "{error}"
    );
}

// ------------------- the standard-library IRI

/// model-toml.md "Standard library": "A json project's `resource` usage
/// of such an IRI is the same usage under the same spelling" -- the
/// publisher-less `kpar-url` form. Without this `sysand add <the IRI>`
/// could not write a `model.toml` entry at all.
#[test]
fn a_json_resource_at_an_omg_iri_becomes_a_publisher_less_kpar_url() {
    let raw = crate::model::InterchangeProjectUsageRaw::Resource {
        resource: "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar".to_owned(),
        version_constraint: None,
    };
    let usage = usage_of_raw(&raw, ".project.json").expect("a standard-library usage");
    assert!(usage.is_stdlib(), "{usage:?}");
    // "the name the KPAR declares" -- which is not the IRI's stem.
    assert_eq!(usage.name().as_str(), "Kernel Semantic Library");
    assert_eq!(usage.identity().publisher, "omg");
    assert_eq!(usage.identity().name, "kernel-semantic-library");
}

/// A well-formed standard-library IRI of a release this Sysand does not
/// embed has no declared name to take, so the stem is what is left --
/// and such an IRI is what `--include-std` exists for.
#[test]
fn an_unembedded_omg_iri_falls_back_to_the_stem() {
    let raw = crate::model::InterchangeProjectUsageRaw::Resource {
        resource: "https://www.omg.org/spec/KerML/20301231/Semantic-Library.kpar".to_owned(),
        version_constraint: None,
    };
    let usage = usage_of_raw(&raw, ".project.json").expect("a standard-library usage");
    assert_eq!(usage.name().as_str(), "Semantic-Library");
}

/// A `kpar-url` usage takes no version constraint,
/// so a json usage that carries one has no manifest form.
#[test]
fn a_constrained_omg_iri_has_no_manifest_form() {
    let raw = crate::model::InterchangeProjectUsageRaw::Resource {
        resource: "https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar".to_owned(),
        version_constraint: Some("^2".to_owned()),
    };
    let error = usage_of_raw(&raw, ".project.json").expect_err("no manifest form");
    assert!(
        error.to_string().contains("takes no version constraint"),
        "{error}"
    );
}

/// The exemption is the IRI's: any other untyped IRI is still refused,
/// because a manifest usage is named by its publisher and name.
#[test]
fn another_untyped_iri_is_still_refused() {
    let raw = crate::model::InterchangeProjectUsageRaw::Resource {
        resource: "https://example.com/lib.kpar".to_owned(),
        version_constraint: None,
    };
    let error = usage_of_raw(&raw, ".project.json").expect_err("no manifest form");
    assert!(
        matches!(&error, NoManifestForm::UntypedIri(iri) if iri == "https://example.com/lib.kpar"),
        "{error}"
    );
}

/// The view's `checksum` is `Some(map)`, never `None` -- a project with
/// no sources gets an *empty* map.
///
/// `kpar_view::meta` writes it unconditionally, and that is deliberate:
/// a `model.toml` project always knows which files it lists, so "no
/// checksums recorded" and "no sources" are the same statement, where a
/// json project's absent `checksum` means neither.
///
/// Pinned because the distinction is invisible from Rust and visible
/// from the bindings, where `Some(empty)` becomes an empty map and
/// `None` becomes null.
#[test]
fn a_source_project_view_carries_an_empty_checksum_map() {
    let text = "manifest-version = \"1.0.0\"\n\n\
                [project]\n\
                publisher = \"acme\"\n\
                name = \"widgets\"\n\
                version = \"1.0.0\"\n\
                sources = []\n\
                metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n";
    let (_info, meta) = super::of_model_toml_in(text, None, |_path| {
        Err::<String, _>("no source is read: the project lists none")
    })
    .expect("the view is built");

    let checksum = meta.checksum.expect("a view always records a checksum map");
    assert!(checksum.is_empty(), "{checksum:?}");
    assert_eq!(meta.index, Some(indexmap::IndexMap::new()));
}
