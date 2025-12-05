// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    env::utils::{CloneError, clone_project},
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectMut, ProjectRead,
        editable::GetPath,
        utils::{ToPathBuf, wrapfs},
    },
};
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Write as _},
};

use camino::{Utf8Path, Utf8PathBuf};
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use thiserror::Error;

use super::utils::{FsIoError, ProjectDeserializationError, ProjectSerializationError};

/// Project stored in a local directory as an extracted kpar archive.
/// Source file paths with (unix) segments `segment1/.../segmentn` are
/// re-interpreted as filesystem-native paths relative to `project_path`.
#[derive(Clone, Debug)]
pub struct LocalSrcProject {
    pub nominal_path: Option<Utf8PathBuf>,
    pub project_path: Utf8PathBuf,
}

impl GetPath for LocalSrcProject {
    fn get_path(&self) -> &str {
        self.project_path.as_str()
    }
}

/// Tries to canonicalise the (longest possible) prefix of a path.
/// Useful if you have /path/to/file/that/does/not/exist
/// but where some prefix, say, /path/to/file can be canonicalised.
fn canonicalise_prefix<P: AsRef<Utf8Path>>(path: P) -> Utf8PathBuf {
    let mut relative_part = Utf8PathBuf::new();
    let mut absolute_part = path.to_path_buf();

    loop {
        if let Ok(canonical_absolute) = absolute_part.canonicalize_utf8() {
            absolute_part = canonical_absolute;
            break;
        }

        match (absolute_part.parent(), absolute_part.file_name()) {
            (Some(absolute_part_parent), Some(absolute_part_file)) => {
                relative_part = Utf8Path::new(absolute_part_file).join(relative_part);
                absolute_part = absolute_part_parent.to_path_buf();
            }
            _ => {
                break;
            }
        }
    }

    absolute_part.push(relative_part);
    absolute_part
}

fn relativise_path<P: AsRef<Utf8Path>, Q: AsRef<Utf8Path>>(
    path: P,
    relative_to: Q,
) -> Option<Utf8PathBuf> {
    let path = if !path.as_ref().is_absolute() {
        let path = camino::absolute_utf8(path.as_ref()).ok()?;
        canonicalise_prefix(path)
    } else {
        canonicalise_prefix(path)
    };

    path.strip_prefix(canonicalise_prefix(relative_to))
        .ok()
        .map(|x| x.to_path_buf())
}

impl LocalSrcProject {
    pub fn root_path(&self) -> Utf8PathBuf {
        self.project_path.clone()
    }

    pub fn info_path(&self) -> Utf8PathBuf {
        self.project_path.join(".project.json")
    }

    pub fn meta_path(&self) -> Utf8PathBuf {
        self.project_path.join(".meta.json")
    }

    pub fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, LocalSrcError> {
        Ok(self.get_project()?.0)
    }

    pub fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, LocalSrcError> {
        Ok(self.get_project()?.1)
    }

    pub fn get_unix_path<P: AsRef<Utf8Path>>(
        &self,
        path: P,
    ) -> Result<Utf8UnixPathBuf, UnixPathError> {
        let root_path = self.root_path();
        let project_path = root_path
            .canonicalize_utf8()
            .map_err(|e| UnixPathError::Canonicalize(root_path, e))?;

        let path = relativise_path(&path, project_path)
            .ok_or_else(|| UnixPathError::PathOutsideProject(path.to_path_buf()))?;

        let mut unix_path = Utf8UnixPathBuf::new();
        for component in path.components() {
            unix_path.push(
                component
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| UnixPathError::Conversion(path.to_owned()))?,
            );
        }

        Ok(unix_path)
    }

    pub fn get_source_path<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Utf8PathBuf, PathError> {
        let utf_path = if path.as_ref().is_absolute() {
            if !cfg!(feature = "lenient_checks") {
                return Err(PathError::AbsolutePath(path.as_ref().to_owned()));
            }
            // This should never fail, as the only way for a Unix path to be absolute is to begin
            // at root /.
            path.as_ref()
                .strip_prefix("/")
                .expect("internal path processing error")
        } else {
            path.as_ref()
        };

        assert!(utf_path.is_relative());

        let mut final_path = self.root_path();
        let mut added_components = 0;
        for component in utf_path.components() {
            match component {
                typed_path::Utf8UnixComponent::RootDir => {
                    unreachable!("root component in a relative path: {utf_path}")
                }
                typed_path::Utf8UnixComponent::CurDir => {}
                typed_path::Utf8UnixComponent::ParentDir => {
                    if added_components > 0 {
                        assert!(final_path.pop());
                        added_components -= 1;
                    } else {
                        return Err(PathError::UnsafePath(
                            utf_path.as_str().into(),
                            typed_path::CheckedPathError::PathTraversalAttack,
                        ));
                    }
                }
                typed_path::Utf8UnixComponent::Normal(component_str) => {
                    final_path.push(component_str);
                    added_components += 1;
                }
            }
        }
        Ok(final_path)
    }

    // TODO: Do we iterate over index or checksum or both?
    pub fn get_source_paths(&self) -> Result<HashSet<Utf8PathBuf>, LocalSrcError> {
        let mut result = HashSet::new();

        // if let Some(meta) = self.get_meta()? {
        //     for index_path in meta.0.keys() {
        //         all_paths.insert(root_path.join(index_path).canonicalize()?);
        //     }

        //     if let Some(checksums) = meta.5 {
        //         for checksum_path in checksums.keys() {
        //             all_paths.insert(root_path.join(checksum_path).canonicalize()?);
        //         }
        //     }
        // }

        // Ok(all_paths)
        if let Some(meta) = self.get_meta()? {
            for path in meta.source_paths(true) {
                let source_path = self.get_source_path(path)?;
                result.insert(source_path);
            }
        };

        Ok(result)
    }

    #[allow(clippy::type_complexity)]
    pub fn temporary_from_project<Pr: ProjectRead>(
        project: &Pr,
    ) -> Result<
        (
            camino_tempfile::Utf8TempDir,
            Self,
            InterchangeProjectInfoRaw,
            InterchangeProjectMetadataRaw,
        ),
        CloneError<Pr::Error, LocalSrcError>,
    > {
        let tmp = camino_tempfile::tempdir().map_err(FsIoError::MkTempDir)?;
        let mut tmp_project = Self {
            nominal_path: None,
            project_path: wrapfs::canonicalize(tmp.path())?,
        };

        let (info, meta) = clone_project(project, &mut tmp_project, true)?;

        Ok((tmp, tmp_project, info, meta))
    }

    // pub fn source_paths(&self) -> &str {
    //     self.get_project()
    // }
}

impl ProjectMut for LocalSrcProject {
    fn put_info(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        let project_json_path = self.info_path();

        if !overwrite && project_json_path.exists() {
            return Err(LocalSrcError::AlreadyExists(
                "`.project.json` already exists".to_string(),
            ));
        }

        let mut file = wrapfs::File::create(&project_json_path)?;
        serde_json::to_writer_pretty(&mut file, info).map_err(|e| {
            ProjectSerializationError::new(
                format!(
                    "failed to serialize and write project info to `{}`",
                    project_json_path
                ),
                e,
            )
        })?;
        file.write(b"\n")
            .map_err(|e| FsIoError::WriteFile(project_json_path, e))?;

        Ok(())
    }

    fn put_meta(
        &mut self,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        let meta_json_path = self.meta_path();
        if !overwrite && meta_json_path.exists() {
            return Err(LocalSrcError::AlreadyExists(
                "'.meta.json' already exists".to_string(),
            ));
        }

        let mut file = wrapfs::File::create(&meta_json_path)?;
        serde_json::to_writer_pretty(&mut file, meta).map_err(|e| {
            ProjectSerializationError::new(
                format!(
                    "failed to serialize and write project metadata to `{}`",
                    meta_json_path
                ),
                e,
            )
        })?;
        file.write(b"\n")
            .map_err(|e| FsIoError::WriteFile(meta_json_path, e))?;

        Ok(())
    }

    fn write_source<P: AsRef<Utf8UnixPath>, R: Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), LocalSrcError> {
        let source_path = self.get_source_path(path)?;

        if !overwrite && source_path.exists() {
            return Err(LocalSrcError::AlreadyExists(format!(
                "`{source_path}` already exists"
            )));
        }

        if let Some(parents) = source_path.parent() {
            wrapfs::create_dir_all(parents)?;
        }

        std::io::copy(source, &mut wrapfs::File::create(&source_path)?)
            .map_err(|e| FsIoError::WriteFile(source_path, e))?;

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum LocalSrcError {
    #[error("{0}")]
    AlreadyExists(String),
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Serialize(#[from] ProjectSerializationError),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl From<FsIoError> for LocalSrcError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

#[derive(Error, Debug)]
pub enum UnixPathError {
    #[error("path `{0}`\n  is outside the project directory")]
    PathOutsideProject(Utf8PathBuf),
    #[error("failed to canonicalize\n  `{0}`:\n  {1}")]
    Canonicalize(Utf8PathBuf, std::io::Error),
    #[error("path `{0}` is not valid Unicode")]
    Conversion(Utf8PathBuf),
}

#[derive(Error, Debug)]
pub enum PathError {
    #[error("path `{0}` is unsafe: {1}")]
    UnsafePath(Utf8PathBuf, typed_path::CheckedPathError),
    #[error("path `{0}` is absolute")]
    AbsolutePath(typed_path::Utf8UnixPathBuf),
}

impl ProjectRead for LocalSrcProject {
    type Error = LocalSrcError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        LocalSrcError,
    > {
        let info_json_path = self.info_path();

        let info_json = if info_json_path.exists() {
            Some(
                serde_json::from_reader(wrapfs::File::open(&info_json_path)?).map_err(|e| {
                    ProjectDeserializationError::new("failed to deserialize `.project.json`", e)
                })?,
            )
        } else {
            None
        };

        let meta_json_path = self.meta_path();

        let meta_json = if meta_json_path.exists() {
            Some(
                serde_json::from_reader(wrapfs::File::open(&meta_json_path)?).map_err(|e| {
                    ProjectDeserializationError::new("failed to deserialize `.meta.json`", e)
                })?,
            )
        } else {
            None
        };

        Ok((info_json, meta_json))
    }

    type SourceReader<'a> = File;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, LocalSrcError> {
        let source_path = self.get_source_path(path)?;

        let f = wrapfs::File::open(&source_path)?;

        Ok(f)
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self.nominal_path.as_ref().map(|p| p.as_str()) {
            Some(path_str) => vec![crate::lock::Source::LocalSrc {
                src_path: path_str.into(),
            }],
            None => vec![],
        }
    }
}
