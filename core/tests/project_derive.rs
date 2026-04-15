// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    collections::HashMap,
    io::{Cursor, Read},
};

use sysand_core::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{CanonicalizationError, ProjectMut, ProjectRead, memory::InMemoryProject},
};

// Have to have these in scope for ProjectRead
// TODO: Find a better solution (that works both inside and outside sysand_core)
use sysand_core::{context::ProjectContext, lock::Source};
use typed_path::Utf8UnixPath;

#[derive(ProjectRead)]
enum OneVariantProjectRead {
    Variant(InMemoryProject),
}

#[derive(ProjectRead)]
enum TwoVariantProjectRead {
    First(InMemoryProject),
    Second(InMemoryProject),
}

#[derive(ProjectRead, ProjectMut)]
enum OneVariantProjectMut {
    Variant(InMemoryProject),
}

#[test]
fn test_macro_one_variant() {
    let _project = OneVariantProjectRead::Variant(InMemoryProject::new());
}

#[test]
fn test_macro_two_variants() {
    let _project_first = TwoVariantProjectRead::First(InMemoryProject::new());
    let _project_second = TwoVariantProjectRead::Second(InMemoryProject::new());
}

#[test]
fn test_error_to_string() {
    let error = <OneVariantProjectRead as ProjectRead>::Error::Variant(
        <InMemoryProject as ProjectRead>::Error::AlreadyExists("project".to_string()),
    );
    let _string = error.to_string();
}

#[test]
fn test_macro_get_project() {
    let info = InterchangeProjectInfoRaw {
        name: "get_project".to_string(),
        publisher: None,
        description: None,
        version: "1.2.3".to_string(),
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    };
    let meta = InterchangeProjectMetadataRaw {
        index: indexmap::IndexMap::new(),
        created: "0000-00-00T00:00:00.123456789Z".to_string(),
        metamodel: None,
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    };
    let test_double = OneVariantProjectRead::Variant(InMemoryProject {
        info: Some(info.clone()),
        meta: Some(meta.clone()),
        files: HashMap::new(),
        nominal_sources: vec![],
    });

    assert_eq!(test_double.get_project().unwrap(), (Some(info), Some(meta)));
}

#[test]
fn test_macro_read_source() {
    let mut files = HashMap::new();
    let path = "path";
    let file_content = "file content".to_string();
    files.insert(path.into(), file_content.clone());
    let project = OneVariantProjectRead::Variant(InMemoryProject {
        info: None,
        meta: None,
        files,
        nominal_sources: vec![],
    });

    let mut buffer = String::new();

    project
        .read_source(path)
        .unwrap()
        .read_to_string(&mut buffer)
        .unwrap();

    assert_eq!(buffer, file_content);
}

#[test]
#[should_panic]
fn test_macro_sources() {
    let project = OneVariantProjectRead::Variant(InMemoryProject::new());

    project.sources(&Default::default()).unwrap();
}

#[test]
fn test_macro_put_info() {
    let info = InterchangeProjectInfoRaw {
        name: "single_get_info".to_string(),
        publisher: None,
        description: None,
        version: "1.2.3".to_string(),
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    };
    let mut project = OneVariantProjectMut::Variant(InMemoryProject::new());

    assert!(project.get_info().unwrap().is_none());

    project.put_info(&info, false).unwrap();

    assert_eq!(project.get_info().unwrap().unwrap(), info);
}

#[test]
fn test_macro_put_meta() {
    let meta = InterchangeProjectMetadataRaw {
        index: indexmap::IndexMap::new(),
        created: "0000-00-00T00:00:00.123456789Z".to_string(),
        metamodel: None,
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    };
    let mut project = OneVariantProjectMut::Variant(InMemoryProject::new());

    assert!(project.get_meta().unwrap().is_none());

    project.put_meta(&meta, false).unwrap();

    assert_eq!(project.get_meta().unwrap().unwrap(), meta);
}

#[test]
fn test_macro_write_source() {
    let path = "path";
    let file_content = "file content".to_string();
    let mut project = OneVariantProjectMut::Variant(InMemoryProject {
        info: None,
        meta: None,
        files: HashMap::new(),
        nominal_sources: vec![],
    });

    project
        .write_source(path, &mut Cursor::new(file_content.as_str()), false)
        .unwrap();

    let mut buffer = String::new();

    project
        .read_source(path)
        .unwrap()
        .read_to_string(&mut buffer)
        .unwrap();

    assert_eq!(buffer, file_content);
}

#[derive(ProjectRead)]
enum GenericProjectRead<SomeProject: ProjectRead> {
    Variant(SomeProject),
}

#[test]
fn test_macro_generic_read() {
    let _project = GenericProjectRead::<InMemoryProject>::Variant(InMemoryProject::new());
}

/// Test double that returns a fixed canonical digest without touching
/// info/meta/sources — mimics a leaf type (e.g. a remote-index-backed
/// project) that supplies a prefetched `project_digest` out of band.
#[derive(Debug)]
struct FixedDigestProject {
    digest: String,
}

impl ProjectRead for FixedDigestProject {
    type Error = std::convert::Infallible;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        // Intentionally return None/None: the test asserts the derived
        // enum returns the *prefetched* digest directly; if the enum
        // falls through to the trait default it would call `get_info`
        // and `canonical_meta` here, see None, and return Ok(None) —
        // which would fail the assertion below.
        Ok((None, None))
    }

    type SourceReader<'a>
        = Cursor<Vec<u8>>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        _path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        Ok(Cursor::new(vec![]))
    }

    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![])
    }

    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        Ok(Some(self.digest.clone()))
    }
}

#[derive(ProjectRead)]
enum WrappingEnum<Inner: ProjectRead> {
    Variant(Inner),
}

#[test]
fn test_derive_forwards_checksum_canonical_hex() {
    // A derived enum must delegate `checksum_canonical_hex` to the active
    // variant. Without this, a leaf type that supplies a prefetched digest
    // (like the remote-index project after versions.json lands) would be
    // bypassed by the trait default's info+canonical_meta fallback.
    let leaf = FixedDigestProject {
        digest: "deadbeef".to_string(),
    };
    let wrapped = WrappingEnum::Variant(leaf);
    let got = wrapped
        .checksum_canonical_hex()
        .expect("forwarded impl must succeed")
        .expect("leaf returns Some");
    assert_eq!(got, "deadbeef");
}

#[derive(ProjectRead, ProjectMut)]
enum GenericProjectMut<SomeProject: ProjectRead + ProjectMut> {
    Variant(SomeProject),
}

#[test]
fn test_macro_generic_mut() {
    let _project = GenericProjectMut::<InMemoryProject>::Variant(InMemoryProject::new());
}
