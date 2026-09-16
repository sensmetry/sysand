// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::str::FromStr as _;

use super::*;
use crate::context::ProjectContext;

fn env_with(projects: &[(&str, &str)]) -> LocalDirectoryEnvironment {
    use std::fmt::Write as _;

    let mut toml = String::from("version = \"0.1\"\n");
    for (iri, version) in projects {
        write!(
            toml,
            r#"
[[project]]
name = "example"
version = "{version}"
path = "lib/example_{version}"
identifiers = ["{iri}"]
"#
        )
        .unwrap();
    }
    LocalDirectoryEnvironment {
        root_dir: Utf8PathBuf::from("/env"),
        metadata: EnvMetadata::from_str(&toml).unwrap(),
    }
}

fn uris_of(env: &LocalDirectoryEnvironment) -> Vec<String> {
    env.uris()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn uris_lists_a_project_once_however_many_versions_are_installed() {
    let env = env_with(&[
        ("urn:kpar:a", "1.0.0"),
        ("urn:kpar:b", "1.0.0"),
        ("urn:kpar:a", "2.0.0"),
    ]);
    assert_eq!(uris_of(&env), ["urn:kpar:a", "urn:kpar:b"]);

    let versions: Vec<String> = env
        .versions("urn:kpar:a")
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(versions, ["1.0.0", "2.0.0"]);
}

/// Builds a real `.sysand` directory holding one installed project, and
/// returns the environment alongside the checksum hashing that project's tree
/// produces.
fn env_with_installed_project(
    root: &Utf8Path,
    recorded_checksum: Option<&str>,
) -> (LocalDirectoryEnvironment, String) {
    let env_dir = root.join(".sysand");
    let project_dir = env_dir.join("lib/example_1.0.0");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(
        project_dir.join(".project.json"),
        r#"{"name":"example","version":"1.0.0"}"#,
    )
    .unwrap();
    std::fs::write(
        project_dir.join(".meta.json"),
        r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#,
    )
    .unwrap();

    let computed = LocalSrcProject::new_access(&project_dir, None)
        .checksum_canonical_hex()
        .unwrap()
        .unwrap();

    let checksum_line = recorded_checksum.map_or_else(String::new, |c| format!("{c}\n"));
    let toml = format!(
        r#"version = "0.1"

[[project]]
name = "example"
version = "1.0.0"
path = "lib/example_1.0.0"
identifiers = ["urn:kpar:example"]
{checksum_line}"#
    );

    let env = LocalDirectoryEnvironment {
        root_dir: env_dir,
        metadata: EnvMetadata::from_str(&toml).unwrap(),
    };
    (env, computed)
}

fn src_checksum_of(env: &LocalDirectoryEnvironment) -> String {
    let project = env.get_project("urn:kpar:example", "1.0.0").unwrap();
    let sources = project.sources(&ProjectContext::default()).unwrap();
    match sources.as_slice() {
        [Source::LocalSrc { checksum, .. }] => checksum.clone(),
        other => panic!("expected a single local src source, got: {other:?}"),
    }
}

/// An environment records the checksum of the source it installed, so
/// `sources()` reports that instead of hashing the whole project tree to
/// arrive at the same value. The reuse is only sound while the two agree.
#[test]
fn installed_project_sources_reuse_the_recorded_src_checksum() {
    let dir = camino_tempfile::tempdir().unwrap();
    let (env, computed) =
        env_with_installed_project(dir.path(), Some(r#"src_cksum = "recorded-not-computed""#));

    assert_eq!(src_checksum_of(&env), "recorded-not-computed");

    // The same fixture without a recorded checksum falls back to hashing, and
    // that is the value a correctly recorded `src_cksum` stands in for.
    let dir = camino_tempfile::tempdir().unwrap();
    let (env, fallback) = env_with_installed_project(dir.path(), None);
    assert_eq!(src_checksum_of(&env), fallback);
    assert_eq!(fallback, computed);
}

/// A `kpar_cksum` digests the archive a project was installed from, not the
/// project tree, so it is not the checksum `sources()` reports and must not be
/// reused as one.
#[test]
fn installed_project_sources_ignore_a_recorded_kpar_checksum() {
    let dir = camino_tempfile::tempdir().unwrap();
    let (env, computed) =
        env_with_installed_project(dir.path(), Some(r#"kpar_cksum = "digest-of-the-archive""#));

    assert_eq!(src_checksum_of(&env), computed);
}

/// Documents the gap the TODO on `get_project_storage` describes: an
/// environment holds every installed version of a project and `EnvResolver`
/// hands all of them to the solver, so resolving through one *is* a version
/// choice and should default a version constraint the way an index does. What
/// comes back is a plain `LocalSrcProject`, which answers for the path it is
/// rather than for the environment that produced it, so a `pkg:sysand` usage
/// met only by the environment admits a prerelease where the index would not.
///
/// Drop the `#[should_panic]` when the environment says so itself.
#[test]
#[should_panic(expected = "an environment offers every installed version")]
fn installed_project_reports_a_source_that_may_offer_multiple_versions() {
    let dir = camino_tempfile::tempdir().unwrap();
    let (env, _computed) = env_with_installed_project(dir.path(), None);

    let project = env.get_project("urn:kpar:example", "1.0.0").unwrap();

    assert!(
        project.source_may_offer_multiple_versions(),
        "an environment offers every installed version of a project, but the \
         project it hands back reports a source that cannot"
    );
}
