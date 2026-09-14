// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::str::FromStr as _;

use super::*;

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
