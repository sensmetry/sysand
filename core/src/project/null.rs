// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{convert::Infallible, io::Read};

use crate::project::{ProjectRead, ProjectReadAsync};

use futures::AsyncRead;
use thiserror::Error;

#[derive(Debug)]
pub struct NullProject {
    nothing: Infallible,
}

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

impl AsyncRead for ImpossibleReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
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
        match self.nothing {}
    }

    type SourceReader<'a>
        = ImpossibleReader
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        _path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self.nothing {}
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self.nothing {}
    }
}

impl ProjectReadAsync for NullProject {
    type Error = NotARealProjectError;

    async fn get_project_async(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        match self.nothing {}
    }

    type SourceReader<'a>
        = ImpossibleReader
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        _path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self.nothing {}
    }

    async fn sources_async(&self) -> Vec<crate::lock::Source> {
        match self.nothing {}
    }
}
