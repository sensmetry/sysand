// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::io::Read;

use thiserror::Error;

use sysand_core::{
    context::ProjectContext,
    lock::Source,
    project::{ProjectChecksum, ProjectMut, ProjectRead, memory::InMemoryProject},
};

pub enum GenericProject<A, B>
where
    A: ProjectRead,
    B: ProjectRead,
{
    Variant1(A),
    Variant2(B),
    Variant3(Box<InMemoryProject>),
}

// What comes after there should essentially be what the ProjectRead and ProjectMut macros expand to,
// so in case the macros are not working properly first make sure this here works.
//
// One deliberate exception: `ProjectMut::put_manifest_document` is not
// forwarded here, where the derive forwards it. The method has a
// refusing default, and leaving it unwritten is what proves that an
// implementation outside this crate compiles without it -- which is the
// reason the default exists. The cost is that
// `Variant3`, an `InMemoryProject` that *can* store a manifest, refuses
// one through this wrapper; `a_hand_written_project_refuses_a_manifest`
// pins exactly that, so the divergence is visible rather than latent.

#[derive(Debug, Error)]
pub enum GenericProjectError<Variant1, Variant2, Variant3> {
    #[error(transparent)]
    Variant1(Variant1),
    #[error(transparent)]
    Variant2(Variant2),
    #[error(transparent)]
    Variant3(Variant3),
}

pub enum GenericProjectSourceReader<Variant1, Variant2, Variant3> {
    Variant1(Variant1),
    Variant2(Variant2),
    Variant3(Variant3),
}

impl<Variant1: Read, Variant2: Read, Variant3: Read> Read
    for GenericProjectSourceReader<Variant1, Variant2, Variant3>
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Variant1(reader) => reader.read(buf),
            Self::Variant2(reader) => reader.read(buf),
            Self::Variant3(reader) => reader.read(buf),
        }
    }
}

impl<A, B> ProjectRead for GenericProject<A, B>
where
    A: ProjectRead,
    B: ProjectRead,
{
    type Error = GenericProjectError<
        <A as ProjectRead>::Error,
        <B as ProjectRead>::Error,
        <InMemoryProject as ProjectRead>::Error,
    >;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<sysand_core::model::InterchangeProjectInfoRaw>,
            Option<sysand_core::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        match self {
            Self::Variant1(project) => project.get_project().map_err(GenericProjectError::Variant1),
            Self::Variant2(project) => project.get_project().map_err(GenericProjectError::Variant2),
            Self::Variant3(project) => project.get_project().map_err(GenericProjectError::Variant3),
        }
    }

    type SourceReader<'a>
        = GenericProjectSourceReader<
        <A as ProjectRead>::SourceReader<'a>,
        <B as ProjectRead>::SourceReader<'a>,
        <InMemoryProject as ProjectRead>::SourceReader<'a>,
    >
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self {
            Self::Variant1(project) => match project.read_source(path) {
                Ok(v) => Ok(GenericProjectSourceReader::Variant1(v)),
                Err(e) => Err(GenericProjectError::Variant1(e)),
            },
            Self::Variant2(project) => match project.read_source(path) {
                Ok(v) => Ok(GenericProjectSourceReader::Variant2(v)),
                Err(e) => Err(GenericProjectError::Variant2(e)),
            },
            Self::Variant3(project) => match project.read_source(path) {
                Ok(v) => Ok(GenericProjectSourceReader::Variant3(v)),
                Err(e) => Err(GenericProjectError::Variant3(e)),
            },
        }
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        match self {
            Self::Variant1(project) => project.sources(ctx).map_err(GenericProjectError::Variant1),
            Self::Variant2(project) => project.sources(ctx).map_err(GenericProjectError::Variant2),
            Self::Variant3(project) => project.sources(ctx).map_err(GenericProjectError::Variant3),
        }
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        match self {
            Self::Variant1(project) => project.source_may_offer_multiple_versions(),
            Self::Variant2(project) => project.source_may_offer_multiple_versions(),
            Self::Variant3(project) => project.source_may_offer_multiple_versions(),
        }
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        match self {
            Self::Variant1(project) => project
                .checksum_canonical_variant()
                .map_err(GenericProjectError::Variant1),
            Self::Variant2(project) => project
                .checksum_canonical_variant()
                .map_err(GenericProjectError::Variant2),
            Self::Variant3(project) => project
                .checksum_canonical_variant()
                .map_err(GenericProjectError::Variant3),
        }
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        match self {
            Self::Variant1(project) => project.project_root(),
            Self::Variant2(project) => project.project_root(),
            Self::Variant3(project) => project.project_root(),
        }
    }
}

impl<A, B> ProjectMut for GenericProject<A, B>
where
    A: ProjectMut,
    B: ProjectMut,
{
    fn put_info(
        &mut self,
        info: &sysand_core::model::InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        match self {
            Self::Variant1(project) => project
                .put_info(info, overwrite)
                .map_err(GenericProjectError::Variant1),
            Self::Variant2(project) => project
                .put_info(info, overwrite)
                .map_err(GenericProjectError::Variant2),
            Self::Variant3(project) => project
                .put_info(info, overwrite)
                .map_err(GenericProjectError::Variant3),
        }
    }
    fn put_meta(
        &mut self,
        meta: &sysand_core::model::InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        match self {
            Self::Variant1(project) => project
                .put_meta(meta, overwrite)
                .map_err(GenericProjectError::Variant1),
            Self::Variant2(project) => project
                .put_meta(meta, overwrite)
                .map_err(GenericProjectError::Variant2),
            Self::Variant3(project) => project
                .put_meta(meta, overwrite)
                .map_err(GenericProjectError::Variant3),
        }
    }
    fn write_source<P: AsRef<typed_path::Utf8UnixPath>, R: Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        match self {
            Self::Variant1(project) => project
                .write_source(path, source, overwrite)
                .map_err(GenericProjectError::Variant1),
            Self::Variant2(project) => project
                .write_source(path, source, overwrite)
                .map_err(GenericProjectError::Variant2),
            Self::Variant3(project) => project
                .write_source(path, source, overwrite)
                .map_err(GenericProjectError::Variant3),
        }
    }
}

#[test]
fn generic_project_create() {
    let _project1 =
        GenericProject::<InMemoryProject, InMemoryProject>::Variant1(InMemoryProject::new());
    let _project2 =
        GenericProject::<InMemoryProject, InMemoryProject>::Variant2(InMemoryProject::new());
    let _project3 = GenericProject::<InMemoryProject, InMemoryProject>::Variant3(Box::new(
        InMemoryProject::new(),
    ));
}

#[test]
fn a_hand_written_project_refuses_a_manifest() {
    use sysand_core::manifest::edit::DocumentMut;
    use sysand_core::project::ManifestWriteError;

    let document: DocumentMut =
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\n"
            .parse()
            .expect("a valid manifest document");

    // The inner project could store one; this wrapper does not forward
    // the method, so the trait's default answers. That an
    // implementation which omits the method compiles is the property
    // under test.
    let mut project: GenericProject<InMemoryProject, InMemoryProject> =
        GenericProject::Variant3(Box::new(InMemoryProject::new()));

    assert!(matches!(
        project.put_manifest_document(&document, false),
        Err(ManifestWriteError::Unsupported)
    ));
}
