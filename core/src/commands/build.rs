use std::path::Path;

use thiserror::Error;

use crate::{
    env::utils::CloneError,
    model::InterchangeProjectValidationError,
    project::{ProjectRead, local_kpar::LocalKParProject},
    project::{local_kpar::IntoKparError, local_src::LocalSrcError, utils::FsIoError},
};

use super::include::IncludeError;

#[derive(Error, Debug)]
pub enum KParBuildError<ProjectReadError> {
    #[error(transparent)]
    ProjectRead(ProjectReadError),
    #[error(transparent)]
    LocalSrc(#[from] LocalSrcError),
    #[error("incomplete project: {0}")]
    IncompleteSource(&'static str),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
    #[error("{0}")]
    Extract(String),
    #[error("unknown file format {0}")]
    UnknownFormat(String),
    #[error("missing project info file '.project.json'")]
    MissingInfo,
    #[error("missing project metadata file '.meta.json'")]
    MissingMeta,
    #[error(transparent)]
    ZipWrite(#[from] zip::result::ZipError),
    #[error("path failure: {0}")]
    PathFailure(String),
    #[error("project serialization error: {0}: {1}")]
    Serialize(&'static str, serde_json::Error),
}

impl<ProjectReadError> From<FsIoError> for KParBuildError<ProjectReadError> {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl<ProjectReadError> From<CloneError<ProjectReadError, LocalSrcError>>
    for KParBuildError<ProjectReadError>
{
    fn from(value: CloneError<ProjectReadError, LocalSrcError>) -> Self {
        match value {
            CloneError::ReadError(error) => Self::ProjectRead(error),
            CloneError::WriteError(error) => error.into(),
            CloneError::IncompleteSource(error) => Self::IncompleteSource(error),
            CloneError::Io(error) => error.into(),
        }
    }
}

impl<ProjectReadError> From<IncludeError<LocalSrcError>> for KParBuildError<ProjectReadError> {
    fn from(value: IncludeError<LocalSrcError>) -> Self {
        match value {
            IncludeError::Project(error) => error.into(),
            IncludeError::Io(error) => error.into(),
            IncludeError::Extract(..) => Self::Extract(value.to_string()),
            IncludeError::UnknownFormat(error) => KParBuildError::UnknownFormat(error),
        }
    }
}

impl<ProjectReadError> From<IntoKparError<LocalSrcError>> for KParBuildError<ProjectReadError> {
    fn from(value: IntoKparError<LocalSrcError>) -> Self {
        match value {
            IntoKparError::MissingInfo => KParBuildError::MissingInfo,
            IntoKparError::MissingMeta => KParBuildError::MissingMeta,
            IntoKparError::ReadError(error) => error.into(),
            IntoKparError::ZipWriteError(zip_error) => zip_error.into(),
            IntoKparError::PathFailure(error) => KParBuildError::PathFailure(error),
            IntoKparError::Io(error) => error.into(),
            IntoKparError::Serialize(msg, e) => Self::Serialize(msg, e),
        }
    }
}

#[cfg(feature = "filesystem")]
pub fn do_build_kpar<P: AsRef<Path>, Pr: ProjectRead>(
    project: &Pr,
    path: P,
    canonicalise: bool,
) -> Result<LocalKParProject, KParBuildError<Pr::Error>> {
    use crate::{model::InterchangeProjectMetadataRaw, project::local_src::LocalSrcProject};

    let building = "Building";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{building:>12}{header:#} kpar: {}",
        path.as_ref().display()
    );

    let (_tmp, mut local_project) = LocalSrcProject::temporary_from_project(project)?;

    if canonicalise {
        for path in local_project
            .get_meta()?
            .unwrap_or_else(InterchangeProjectMetadataRaw::generate_blank)
            .validate()?
            .source_paths(true)
        {
            use crate::include::do_include;

            do_include(&mut local_project, &path, true, true, None)?;
        }
    }

    Ok(LocalKParProject::from_project(&local_project, path)?)
}
