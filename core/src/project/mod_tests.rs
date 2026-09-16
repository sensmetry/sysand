// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::collections::HashMap;

use indexmap::IndexMap;
use typed_path::Utf8UnixPath;

use crate::{
    context::ProjectContext,
    lock::Source,
    manifest::edit::DocumentMut,
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        KerMlChecksumAlg,
    },
    project::{
        ManifestError, ManifestWriteError, ProjectChecksum, ProjectFormat, ProjectMut, ProjectRead,
        hash_reader,
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
        model_toml: None,
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

/// A project that keeps the default `format` answers json without
/// saying anything.
///
/// The vehicle has to be a project that says nothing. `InMemoryProject`
/// is not one: it can hold a `model.toml`, so it answers from what it
/// holds -- and an empty one holds neither manifest.
#[test]
fn a_project_that_does_not_say_is_json() {
    assert_eq!(Silent.format().expect("infallible"), ProjectFormat::Json);
}

/// A project that says nothing about its format: it keeps the default
/// `format`.
#[derive(Debug)]
struct Silent;

impl ProjectRead for Silent {
    type Error = InMemoryError;

    fn source_may_offer_multiple_versions(&self) -> bool {
        false
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

    type SourceReader<'a> = &'a [u8];

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        _path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        Ok(&[])
    }

    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![])
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        Err(InMemoryError::MissingInfoMeta)
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        None
    }
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

    fn source_may_offer_multiple_versions(&self) -> bool {
        false
    }

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
    Json(ProjectReference<Silent>),
    Toml(Toml),
}

/// The `ProjectRead` derive forwards `format` to the active variant.
#[test]
fn a_derived_enum_reports_the_format_of_its_variant() {
    assert_eq!(
        Either::Json(ProjectReference::new(Silent))
            .format()
            .expect("infallible"),
        ProjectFormat::Json
    );
    assert_eq!(
        Either::Toml(Toml).format().expect("infallible"),
        ProjectFormat::Toml
    );
}

fn a_manifest() -> DocumentMut {
    "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\n"
        .parse()
        .expect("a valid manifest document")
}

/// Stores a manifest through the trait, so that `P = &mut T` picks the
/// blanket implementation rather than resolving to the inner type by
/// auto-deref.
fn store<P: ProjectMut>(
    mut project: P,
    document: &DocumentMut,
) -> Result<(), ManifestWriteError<P::Error>> {
    project.put_manifest_document(document, false)
}

#[test]
fn a_mutable_reference_forwards_a_manifest_write() {
    // Silent without the blanket impl's override: the call falls
    // through to the trait default and answers `Unsupported` for a
    // project that can store one.
    let mut project = InMemoryProject::default();

    store(&mut project, &a_manifest()).expect("an in-memory project stores a manifest");

    assert_eq!(project.format().expect("a format"), ProjectFormat::Toml);
}

#[test]
fn a_project_without_a_manifest_has_no_document() {
    assert!(matches!(
        Silent.manifest_document(),
        Err(ManifestError::NoTomlManifest)
    ));
}

#[test]
fn a_memory_project_hands_back_its_manifest_document() {
    // Comments included: what an editor needs is what the file says,
    // not the values `manifest()` resolves out of it.
    let commented = "manifest-version = \"1.0.0\"\n\n[project]\n# who publishes it\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\n";
    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(&commented.parse().expect("a manifest"), false)
        .expect("stores a manifest");

    let document = project.manifest_document().expect("a document");
    assert!(document.to_string().contains("# who publishes it"));
}

#[test]
fn a_manifest_this_sysand_cannot_read_is_refused_before_it_is_edited() {
    // Parsing TOML is not reading a manifest. A file written for a
    // version this Sysand does not support parses perfectly well and
    // must not be edited.
    let newer = "manifest-version = \"2.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\n";
    let project = InMemoryProject {
        model_toml: Some(newer.to_owned()),
        ..InMemoryProject::default()
    };

    let error = project
        .manifest_document()
        .expect_err("a manifest from the future is not editable");
    assert!(error.to_string().contains("manifest-version 2"));
}

#[test]
fn a_mutable_reference_forwards_the_manifest_document() {
    fn document<P: ProjectRead>(project: P) -> Result<DocumentMut, ManifestError<P::Error>> {
        project.manifest_document()
    }

    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(&a_manifest(), false)
        .expect("stores a manifest");

    document(&project).expect("a reference forwards the manifest document");
}
