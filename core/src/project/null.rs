// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    convert::Infallible,
    io::{self, Read},
    pin::Pin,
};

use futures::AsyncRead;
use thiserror::Error;

use crate::{
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectRead, ProjectReadAsync},
};

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
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        match self.nothing {}
    }
}

impl AsyncRead for ImpossibleReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match self.nothing {}
    }
}

impl ProjectRead for NullProject {
    type Error = NotARealProjectError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
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

    fn sources(&self) -> Vec<Source> {
        match self.nothing {}
    }
}

impl ProjectReadAsync for NullProject {
    type Error = NotARealProjectError;

    async fn get_project_async(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
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

    async fn sources_async(&self) -> Vec<Source> {
        match self.nothing {}
    }
}
