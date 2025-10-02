#[cfg(feature = "filesystem")]
use std::path::Path;

#[cfg(feature = "filesystem")]
use thiserror::Error;

#[cfg(feature = "filesystem")]
use crate::{
    model::InterchangeProjectValidationError,
    project::{ProjectRead, local_kpar::LocalKParProject},
};

#[cfg(feature = "filesystem")]
#[derive(Error, Debug)]
pub enum KParBuildError<ProjectReadError> {
    #[error(transparent)]
    ProjectRead(ProjectReadError),
    #[error(transparent)]
    LocalSrc(#[from] crate::project::local_src::LocalSrcError),
    #[error("incomplete sources {0}")]
    IncompleteSource(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
    #[error(transparent)]
    Extract(#[from] crate::symbols::ExtractError),
    #[error("unknown format {0}")]
    UnknownFormat(String),
    #[error("missing project info")]
    MissingInfo,
    #[error("missing project metadata")]
    MissingMeta,
    #[error("{0}")]
    ZipWrite(#[from] zip::result::ZipError),
    #[error("path failure: {0}")]
    PathFailure(String),
    #[error("invalid filename")]
    InvalidFileName,
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

// impl<ProjectReadError> From<...> for KParBuildError<ProjectReadError> {
// }

#[cfg(feature = "filesystem")]
impl<ProjectReadError>
    From<crate::env::utils::CloneError<ProjectReadError, crate::project::local_src::LocalSrcError>>
    for KParBuildError<ProjectReadError>
{
    fn from(
        value: crate::env::utils::CloneError<
            ProjectReadError,
            crate::project::local_src::LocalSrcError,
        >,
    ) -> Self {
        match value {
            crate::env::utils::CloneError::ReadError(error) => KParBuildError::ProjectRead(error),
            crate::env::utils::CloneError::WriteError(error) => error.into(),
            crate::env::utils::CloneError::IncompleteSourceError(error) => {
                KParBuildError::IncompleteSource(error)
            }
            crate::env::utils::CloneError::IOError(error) => error.into(),
        }
    }
}

#[cfg(feature = "filesystem")]
impl<ProjectReadError> From<super::include::IncludeError<crate::project::local_src::LocalSrcError>>
    for KParBuildError<ProjectReadError>
{
    fn from(value: super::include::IncludeError<crate::project::local_src::LocalSrcError>) -> Self {
        match value {
            super::include::IncludeError::Project(error) => error.into(),
            super::include::IncludeError::Io(error) => error.into(),
            super::include::IncludeError::Extract(extract_error) => extract_error.into(),
            super::include::IncludeError::UnknownFormat(error) => {
                KParBuildError::UnknownFormat(error)
            }
        }
    }
}

#[cfg(feature = "filesystem")]
impl<ProjectReadError>
    From<crate::project::local_kpar::IntoKparError<crate::project::local_src::LocalSrcError>>
    for KParBuildError<ProjectReadError>
{
    fn from(
        value: crate::project::local_kpar::IntoKparError<crate::project::local_src::LocalSrcError>,
    ) -> Self {
        match value {
            crate::project::local_kpar::IntoKparError::MissingInfo => KParBuildError::MissingInfo,
            crate::project::local_kpar::IntoKparError::MissingMeta => KParBuildError::MissingMeta,
            crate::project::local_kpar::IntoKparError::ReadError(error) => error.into(),
            crate::project::local_kpar::IntoKparError::ZipWriteError(zip_error) => zip_error.into(),
            crate::project::local_kpar::IntoKparError::PathFailure(error) => {
                KParBuildError::PathFailure(error)
            }
            crate::project::local_kpar::IntoKparError::IOError(error) => error.into(),
            crate::project::local_kpar::IntoKparError::FileNameError => {
                KParBuildError::InvalidFileName
            }
            crate::project::local_kpar::IntoKparError::SerdeError(error) => error.into(),
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
