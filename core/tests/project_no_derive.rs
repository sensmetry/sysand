// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::Read;

use thiserror::Error;

use sysand_core::project::{ProjectMut, ProjectRead, memory::InMemoryProject};

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
            GenericProjectSourceReader::Variant1(reader) => reader.read(buf),
            GenericProjectSourceReader::Variant2(reader) => reader.read(buf),
            GenericProjectSourceReader::Variant3(reader) => reader.read(buf),
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
            GenericProject::Variant1(project) => {
                project.get_project().map_err(GenericProjectError::Variant1)
            }
            GenericProject::Variant2(project) => {
                project.get_project().map_err(GenericProjectError::Variant2)
            }
            GenericProject::Variant3(project) => {
                project.get_project().map_err(GenericProjectError::Variant3)
            }
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
            GenericProject::Variant1(project) => project
                .read_source(path)
                .map(GenericProjectSourceReader::Variant1)
                .map_err(GenericProjectError::Variant1),
            GenericProject::Variant2(project) => project
                .read_source(path)
                .map(GenericProjectSourceReader::Variant2)
                .map_err(GenericProjectError::Variant2),
            GenericProject::Variant3(project) => project
                .read_source(path)
                .map(GenericProjectSourceReader::Variant3)
                .map_err(GenericProjectError::Variant3),
        }
    }

    fn sources(&self) -> Vec<sysand_core::lock::Source> {
        match self {
            GenericProject::Variant1(project) => project.sources(),
            GenericProject::Variant2(project) => project.sources(),
            GenericProject::Variant3(project) => project.sources(),
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
            GenericProject::Variant1(project) => project
                .put_info(info, overwrite)
                .map_err(GenericProjectError::Variant1),
            GenericProject::Variant2(project) => project
                .put_info(info, overwrite)
                .map_err(GenericProjectError::Variant2),
            GenericProject::Variant3(project) => project
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
            GenericProject::Variant1(project) => project
                .put_meta(meta, overwrite)
                .map_err(GenericProjectError::Variant1),
            GenericProject::Variant2(project) => project
                .put_meta(meta, overwrite)
                .map_err(GenericProjectError::Variant2),
            GenericProject::Variant3(project) => project
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
            GenericProject::Variant1(project) => project
                .write_source(path, source, overwrite)
                .map_err(GenericProjectError::Variant1),
            GenericProject::Variant2(project) => project
                .write_source(path, source, overwrite)
                .map_err(GenericProjectError::Variant2),
            GenericProject::Variant3(project) => project
                .write_source(path, source, overwrite)
                .map_err(GenericProjectError::Variant3),
        }
    }
}

#[test]
fn test_basic() {
    let _project1 =
        GenericProject::<InMemoryProject, InMemoryProject>::Variant1(InMemoryProject::new());
    let _project2 =
        GenericProject::<InMemoryProject, InMemoryProject>::Variant2(InMemoryProject::new());
    let _project3 = GenericProject::<InMemoryProject, InMemoryProject>::Variant3(Box::new(
        InMemoryProject::new(),
    ));
}
