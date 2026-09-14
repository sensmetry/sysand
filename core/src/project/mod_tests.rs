// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::collections::HashMap;

use indexmap::IndexMap;
use typed_path::Utf8UnixPath;

use crate::{
    context::ProjectContext,
    lock::Source,
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        KerMlChecksumAlg,
    },
    project::{
        ProjectChecksum, ProjectFormat, ProjectRead, hash_reader,
        memory::{InMemoryError, InMemoryProject},
        reference::ProjectReference,
    },
    utils::lowercase_hex,
};

#[test]
fn sanity_check_hasher() -> Result<(), Box<dyn std::error::Error>> {
    let input = "FooBarBaz";

    // echo -n "FooBarBaz" | sha256sum
    assert_eq!(
        lowercase_hex(hash_reader(&mut std::io::Cursor::new(input))?),
        "4da8b89a905445e96dd0ab6c9be9a72c8b0ffc686a57a3cc6808a8952a3560ed"
    );

    Ok(())
}

#[test]
fn canonicalization_no_checksums() -> Result<(), Box<dyn std::error::Error>> {
    let project = InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: "canonicalization".to_owned(),
            publisher: None,
            description: None,
            version: "1.2.3".to_owned(),
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        }),
        meta: Some(InterchangeProjectMetadataRaw {
            index: IndexMap::default(),
            created: "123".to_owned(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: Some(IndexMap::from([(
                "MyFile.txt".to_owned(),
                InterchangeProjectChecksumRaw {
                    algorithm: KerMlChecksumAlg::None.to_string(),
                    value: String::new(),
                },
            )])),
        }),
        files: HashMap::from([(
            Utf8UnixPath::new("MyFile.txt").to_path_buf(),
            "FooBarBaz".to_owned(),
        )]),
        nominal_sources: vec![],
        source_may_offer_multiple_versions: false,
    };

    let Some(canonical_info) = project.canonical_meta()? else {
        panic!()
    };

    let Some(checksums) = canonical_info.checksum else {
        panic!()
    };

    assert_eq!(checksums.len(), 1);
    assert_eq!(
        checksums.get("MyFile.txt"),
        Some(&InterchangeProjectChecksumRaw {
            value: "4da8b89a905445e96dd0ab6c9be9a72c8b0ffc686a57a3cc6808a8952a3560ed".to_owned(),
            algorithm: KerMlChecksumAlg::Sha256.to_string()
        })
    );

    Ok(())
}

/// Every project that has only ever had the two json documents answers
/// json without saying anything.
#[test]
fn a_project_that_does_not_say_is_json() {
    assert_eq!(
        InMemoryProject::new().format().expect("infallible"),
        ProjectFormat::Json
    );
}

/// A wrapper must forward rather than take the default, or it claims
/// json for whatever it is holding. `ProjectReference` is the `Arc`
/// wrapper, so the claim would survive being shared.
#[test]
fn a_reference_reports_the_format_of_what_it_wraps() {
    let wrapped = ProjectReference::new(Toml);
    assert_eq!(wrapped.format().expect("infallible"), ProjectFormat::Toml);
}

/// A project that says it is toml, with nothing else in it. The point of
/// the tests below is the forwarding, not the reading.
#[derive(Debug)]
struct Toml;

impl ProjectRead for Toml {
    type Error = InMemoryError;

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        Ok(ProjectFormat::Toml)
    }

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        Ok((None, None))
    }

    type SourceReader<'a> = std::io::Cursor<String>;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        _path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        Ok(std::io::Cursor::new(String::new()))
    }

    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![])
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        None
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        Ok(ProjectChecksum::Project(String::new()))
    }
}

/// The json side is behind a `ProjectReference` so that the two variants
/// are a similar size; it also means the derive forwards through a
/// wrapper.
#[derive(crate::project::ProjectRead)]
enum Either {
    Json(ProjectReference<InMemoryProject>),
    Toml(Toml),
}

/// The `ProjectRead` derive forwards `format` to the active variant.
#[test]
fn a_derived_enum_reports_the_format_of_its_variant() {
    assert_eq!(
        Either::Json(ProjectReference::new(InMemoryProject::new()))
            .format()
            .expect("infallible"),
        ProjectFormat::Json
    );
    assert_eq!(
        Either::Toml(Toml).format().expect("infallible"),
        ProjectFormat::Toml
    );
}
