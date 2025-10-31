// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{convert::Infallible, io::Read};

use crate::project::ProjectRead;

use thiserror::Error;

#[derive(Debug)]
pub struct NullProject {}

#[derive(Error, Debug)]
pub enum NotARealProjectError {
    #[error("null project error")]
    NotARealProject,
}

pub struct ImpossibleReader {
    nothing: Infallible,
}

impl Read for ImpossibleReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        match self.nothing {}
    }
}

impl ProjectRead for NullProject {
    type Error = NotARealProjectError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        Err(NotARealProjectError::NotARealProject)
    }

    type SourceReader<'a>
        = ImpossibleReader
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        _path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        Err(NotARealProjectError::NotARealProject)
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        vec![]
    }
}
