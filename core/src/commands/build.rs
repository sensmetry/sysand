use camino::Utf8Path;
use thiserror::Error;

use crate::{
    env::utils::{CloneError, ErrorBound},
    include::IncludeError,
    model::InterchangeProjectValidationError,
    project::{
        ProjectRead,
        local_kpar::{IntoKparError, LocalKParProject},
        local_src::{LocalSrcError, LocalSrcProject},
        utils::{FsIoError, ZipArchiveError},
    },
    workspace::{Workspace, WorkspaceReadError},
};

#[derive(Default, Copy, Clone, Debug, PartialEq, Eq)]
// Currently python interop is done with strings instead
// in part to have less boilerplate, in part because the old
// Python we use doesn't have pattern matching which ensures
// all cases are covered
// #[cfg_attr(feature = "python", pyclass(eq))]
pub enum KparCompressionMethod {
    /// Store the files as is
    Stored,
    /// Compress the files using Deflate
    #[default]
    Deflated,
    /// Compress the files using BZIP2
    #[cfg(feature = "kpar-bzip2")]
    Bzip2,
    /// Compress the files using ZStandard
    #[cfg(feature = "kpar-zstd")]
    Zstd,
    /// Compress the files using XZ
    #[cfg(feature = "kpar-xz")]
    Xz,
    /// Compress the files using PPMd
    #[cfg(feature = "kpar-ppmd")]
    Ppmd,
}

impl From<KparCompressionMethod> for zip::CompressionMethod {
    fn from(value: KparCompressionMethod) -> Self {
        match value {
            KparCompressionMethod::Stored => zip::CompressionMethod::Stored,
            KparCompressionMethod::Deflated => zip::CompressionMethod::Deflated,
            #[cfg(feature = "kpar-bzip2")]
            KparCompressionMethod::Bzip2 => zip::CompressionMethod::Bzip2,
            #[cfg(feature = "kpar-zstd")]
            KparCompressionMethod::Zstd => zip::CompressionMethod::Zstd,
            #[cfg(feature = "kpar-xz")]
            KparCompressionMethod::Xz => zip::CompressionMethod::Xz,
            #[cfg(feature = "kpar-ppmd")]
            KparCompressionMethod::Ppmd => zip::CompressionMethod::Ppmd,
        }
    }
}

#[derive(Debug, Error)]
pub enum CompressionMethodParseError {
    #[error("Compile sysand with feature {feature} to use {compression} compression")]
    SuggestFeature {
        compression: String,
        feature: String,
    },
    #[error("{0}")]
    Invalid(String),
}

impl TryFrom<String> for KparCompressionMethod {
    type Error = CompressionMethodParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl TryFrom<&str> for KparCompressionMethod {
    type Error = CompressionMethodParseError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "STORED" => Ok(KparCompressionMethod::Stored),
            "DEFLATED" => Ok(KparCompressionMethod::Deflated),
            #[cfg(feature = "kpar-bzip2")]
            "BZIP2" => Ok(KparCompressionMethod::Bzip2),
            #[cfg(not(feature = "kpar-bzip2"))]
            "BZIP2" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-bzip2".into(),
            }),
            #[cfg(feature = "kpar-zstd")]
            "ZSTD" => Ok(KparCompressionMethod::Zstd),
            #[cfg(not(feature = "kpar-zstd"))]
            "ZSTD" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-zstd".into(),
            }),
            #[cfg(feature = "kpar-xz")]
            "XZ" => Ok(KparCompressionMethod::Xz),
            #[cfg(not(feature = "kpar-xz"))]
            "XZ" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-xz".into(),
            }),
            #[cfg(feature = "kpar-ppmd")]
            "PPMD" => Ok(KparCompressionMethod::Ppmd),
            #[cfg(not(feature = "kpar-ppmd"))]
            "PPMD" => Err(CompressionMethodParseError::SuggestFeature {
                compression: value.into(),
                feature: "kpar-ppmd".into(),
            }),
            _ => Err(CompressionMethodParseError::Invalid(format!(
                "Compression method `{value}` is invalid"
            ))),
        }
    }
}

#[derive(Error, Debug)]
pub enum KParBuildError<ProjectReadError: ErrorBound> {
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
    #[error(
        "project includes a path usage `{0}`,\n\
        which is unlikely to be available on other computers at the same path"
    )]
    PathUsage(String),
    #[error(
        "workspace sets metamodel `{workspace_metamodel}`, but project `{project_path}` \
         sets a different metamodel `{project_metamodel}` in `.meta.json`;\n\
         remove the metamodel from the project's `.meta.json` or from `.workspace.json`"
    )]
    WorkspaceMetamodelConflict {
        workspace_metamodel: String,
        project_metamodel: String,
        project_path: String,
    },
}

impl<ProjectReadError: ErrorBound> From<FsIoError> for KParBuildError<ProjectReadError> {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl<ProjectReadError: ErrorBound> From<CloneError<ProjectReadError, LocalSrcError>>
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

impl<ProjectReadError: ErrorBound> From<IncludeError<LocalSrcError>>
    for KParBuildError<ProjectReadError>
{
    fn from(value: IncludeError<LocalSrcError>) -> Self {
        match value {
            IncludeError::Project(error) => error.into(),
            IncludeError::Io(error) => error.into(),
            IncludeError::Extract(..) => Self::Extract(value.to_string()),
            IncludeError::UnknownFormat(error) => KParBuildError::UnknownFormat(error),
        }
    }
}

impl<ProjectReadError: ErrorBound> From<IntoKparError<LocalSrcError>>
    for KParBuildError<ProjectReadError>
{
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

pub fn do_build_kpar<P: AsRef<Utf8Path>, Pr: ProjectRead>(
    project: &Pr,
    path: P,
    compression: KparCompressionMethod,
    canonicalise: bool,
    allow_path_usage: bool,
) -> Result<LocalKParProject, KParBuildError<Pr::Error>> {
    do_build_kpar_inner(
        project,
        path,
        compression,
        canonicalise,
        allow_path_usage,
        None,
    )
}

fn do_build_kpar_inner<P: AsRef<Utf8Path>, Pr: ProjectRead>(
    project: &Pr,
    path: P,
    compression: KparCompressionMethod,
    canonicalise: bool,
    allow_path_usage: bool,
    workspace_metamodel: Option<&str>,
) -> Result<LocalKParProject, KParBuildError<Pr::Error>> {
    use crate::project::local_src::LocalSrcProject;

    let building = "Building";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{building:>12}{header:#} kpar `{}`", path.as_ref());

    let (_tmp, mut local_project, info, mut meta) =
        LocalSrcProject::temporary_from_project(project)?;
    match semver::Version::parse(&info.version) {
        Ok(_) => (),
        Err(e) => log::warn!(
            "project's version `{}` is not a valid SemVer version: {e}",
            info.version
        ),
    }
    if let Some(l) = info.license {
        match spdx::Expression::parse(&l) {
            Ok(_) => (),
            Err(e) => {
                log::warn!("project's license `{l}` is not a valid SPDX license expression:\n{e}")
            }
        }
    }

    if let Some(u) = info.usage.iter().find(|x| {
        // Case-insensitively match `file:` scheme
        x.resource.len() >= 5
            && x.resource
                .as_bytes()
                .iter()
                .zip(b"file:")
                .all(|(c1, &c2)| c1.to_ascii_lowercase() == c2)
    }) {
        if allow_path_usage {
            log::warn!(
                "project includes a path usage `{}`,\n\
            which is unlikely to be available on other computers at the same path",
                u.resource
            );
        } else {
            return Err(KParBuildError::PathUsage(u.resource.clone()));
        }
    }

    if let Some(ws_metamodel) = workspace_metamodel {
        if let Some(proj_metamodel) = &meta.metamodel {
            if proj_metamodel != ws_metamodel {
                return Err(KParBuildError::WorkspaceMetamodelConflict {
                    workspace_metamodel: ws_metamodel.to_string(),
                    project_metamodel: proj_metamodel.clone(),
                    project_path: path.as_ref().to_string(),
                });
            }
        } else {
            meta.metamodel = Some(ws_metamodel.to_string());
            use crate::project::ProjectMut;
            local_project
                .put_meta(&meta, true)
                .map_err(KParBuildError::from)?;
        }
    }

    if canonicalise {
        for path in meta.validate()?.source_paths(true) {
            use crate::include::do_include;

            do_include(&mut local_project, &path, true, true, None)?;
        }
    }

    Ok(LocalKParProject::from_project(
        &local_project,
        path,
        compression.into(),
    )?)
}

pub fn do_build_workspace_kpars<P: AsRef<Utf8Path>>(
    workspace: &Workspace,
    path: P,
    compression: KparCompressionMethod,
    canonicalise: bool,
    allow_path_usage: bool,
) -> Result<Vec<LocalKParProject>, KParBuildError<LocalSrcError>> {
    let ws_metamodel = workspace.metamodel().map(|iri| iri.as_str());

    let mut result = Vec::new();
    for project_root in workspace.projects() {
        let project = LocalSrcProject {
            nominal_path: None,
            project_path: workspace.root_path().join(&project_root.path),
        };

        let file_name = default_kpar_file_name(&project)?;
        let output_path = path.as_ref().join(file_name);
        let kpar_project = do_build_kpar_inner(
            &project,
            &output_path,
            compression,
            canonicalise,
            allow_path_usage,
            ws_metamodel,
        )?;
        result.push(kpar_project);
    }
    Ok(result)
}
