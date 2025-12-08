use std::path::Path;

use thiserror::Error;

use crate::{
    env::utils::CloneError,
    model::InterchangeProjectValidationError,
    project::{
        ProjectRead,
        local_kpar::{IntoKparError, LocalKParProject},
        local_src::LocalSrcError,
        utils::{FsIoError, ZipArchiveError},
    },
    workspace::WorkspaceReadError,
};
#[cfg(feature = "filesystem")]
use crate::{project::local_src::LocalSrcProject, workspace::Workspace};

use super::include::IncludeError;

#[derive(Error, Debug)]
pub enum KParBuildError<ProjectReadError> {
    #[error(transparent)]
    ProjectRead(ProjectReadError),
    #[error(transparent)]
    WorkspaceRead(#[from] WorkspaceReadError),
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
    #[error(
        "unknown file format of '{0}', only SysML v2 (.sysml) and KerML (.kerml) files are supported"
    )]
    UnknownFormat(Box<str>),
    #[error("missing project info file `.project.json`")]
    MissingInfo,
    #[error("missing project metadata file `.meta.json`")]
    MissingMeta,
    #[error(transparent)]
    Zip(#[from] ZipArchiveError),
    #[error("project serialization error: {0}: {1}")]
    Serialize(&'static str, serde_json::Error),
    #[error("internal error: {0}")]
    InternalError(&'static str),
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
            CloneError::ProjectRead(error) => Self::ProjectRead(error),
            CloneError::EnvWrite(error) => error.into(),
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
            IntoKparError::ProjectRead(error) => error.into(),
            IntoKparError::Zip(zip_error) => zip_error.into(),
            IntoKparError::Io(error) => error.into(),
            IntoKparError::Serialize(msg, e) => Self::Serialize(msg, e),
        }
    }
}

#[cfg(feature = "filesystem")]
pub fn default_kpar_file_name<Pr: ProjectRead>(
    project: &Pr,
) -> Result<String, KParBuildError<Pr::Error>> {
    let Some(project_info) = project.get_info().map_err(KParBuildError::ProjectRead)? else {
        return Err(KParBuildError::MissingInfo);
    };
    Ok(format!(
        "{}-{}.kpar",
        project_info
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>(),
        project_info.version
    ))
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
        "{header}{building:>12}{header:#} kpar `{}`",
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

#[cfg(feature = "filesystem")]
pub fn do_build_workspace_kpars<P: AsRef<Path>>(
    workspace: &Workspace,
    path: P,
    canonicalise: bool,
) -> Result<Vec<LocalKParProject>, KParBuildError<LocalSrcError>> {
    let mut result = Vec::new();
    let Some(projects) = workspace.get_projects()? else {
        // The caller should have already checked that the .workspace.json file
        // exists.
        return Err(KParBuildError::InternalError("missing .workspace.json."));
    };
    for project in projects {
        let project = LocalSrcProject {
            project_path: workspace.workspace_path.join(project.path),
        };
        let file_name = default_kpar_file_name(&project)?;
        let output_path = path.as_ref().join(file_name);
        let kpar_project = do_build_kpar(&project, &output_path, canonicalise)?;
        result.push(kpar_project);
    }
    Ok(result)
}
