// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::io::Read;
use sysand_core::project::VersionAndUsage;

use thiserror::Error;

use sysand_core::{
    context::ProjectContext,
    lock::Source,
    manifest::edit::DocumentMut,
    project::{
        ProjectChecksum, ProjectDefinition, ProjectMut, ProjectRead, memory::InMemoryProject,
    },
};

#[derive(Debug)]
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
// implementation outside this crate compiles without it, which is the
// reason the default exists. The cost is that
// `Variant3`, an `InMemoryProject` that *can* store a manifest, refuses
// one through this wrapper; `a_hand_written_project_refuses_a_manifest`
// pins exactly that.

#[derive(Debug, Error)]
pub enum GenericProjectError<Variant1, Variant2, Variant3> {
    #[error(transparent)]
    Variant1(Variant1),
    #[error(transparent)]
    Variant2(Variant2),
    #[error(transparent)]
    Variant3(Variant3),
}

#[derive(Debug)]
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

    fn format(&self) -> Result<sysand_core::project::ProjectFormat, Self::Error> {
        match self {
            Self::Variant1(project) => project.format().map_err(GenericProjectError::Variant1),
            Self::Variant2(project) => project.format().map_err(GenericProjectError::Variant2),
            Self::Variant3(project) => project.format().map_err(GenericProjectError::Variant3),
        }
    }

    fn manifest_document(
        &self,
    ) -> Result<DocumentMut, sysand_core::project::ManifestError<Self::Error>> {
        match self {
            Self::Variant1(project) => project
                .manifest_document()
                .map_err(|e| e.map_project_read(GenericProjectError::Variant1)),
            Self::Variant2(project) => project
                .manifest_document()
                .map_err(|e| e.map_project_read(GenericProjectError::Variant2)),
            Self::Variant3(project) => project
                .manifest_document()
                .map_err(|e| e.map_project_read(GenericProjectError::Variant3)),
        }
    }

    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
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

    fn version_and_usage(&self) -> Result<Option<VersionAndUsage>, Self::Error> {
        match self {
            Self::Variant1(project) => project
                .version_and_usage()
                .map_err(GenericProjectError::Variant1),
            Self::Variant2(project) => project
                .version_and_usage()
                .map_err(GenericProjectError::Variant2),
            Self::Variant3(project) => project
                .version_and_usage()
                .map_err(GenericProjectError::Variant3),
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
    fn put_manifest_document(
        &mut self,
        document: &DocumentMut,
        overwrite: bool,
    ) -> Result<(), sysand_core::project::ManifestWriteError<Self::Error>> {
        match self {
            Self::Variant1(project) => project
                .put_manifest_document(document, overwrite)
                .map_err(|e| e.map_project_write(GenericProjectError::Variant1)),
            Self::Variant2(project) => project
                .put_manifest_document(document, overwrite)
                .map_err(|e| e.map_project_write(GenericProjectError::Variant2)),
            Self::Variant3(project) => project
                .put_manifest_document(document, overwrite)
                .map_err(|e| e.map_project_write(GenericProjectError::Variant3)),
        }
    }

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
