// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    env::utils::{CloneError, clone_project},
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectMut, ProjectRead},
};
use std::{collections::HashSet, fs::File, io::Read, path::PathBuf};

use tempfile::tempdir;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use thiserror::Error;

/// Project stored in a local directory as an extracted kpar archive.
/// Source file paths with (unix) segments segment1/.../segmentn are
/// re-interpreted as filesystem-native paths relative to `project_path`.
#[derive(Clone, Debug)]
pub struct LocalSrcProject {
    pub project_path: PathBuf,
}

// Tries to canonicalise the (longest possible) prefix of a path.
// Useful if you have /path/to/file/that/does/not/exist
// but where some prefix, say, /path/to/file can be canonicalised.
fn canonicalise_prefix<P: AsRef<std::path::Path>>(path: P) -> PathBuf {
    let mut relative_part = PathBuf::new();
    let mut absolute_part = path.as_ref().to_path_buf();

    loop {
        if let Ok(canonical_absolute) = absolute_part.canonicalize() {
            absolute_part = canonical_absolute;
            break;
        }

        match (absolute_part.parent(), absolute_part.file_name()) {
            (Some(absolute_part_parent), Some(absolute_part_file)) => {
                relative_part = PathBuf::from(absolute_part_file).join(relative_part);
                absolute_part = absolute_part_parent.to_path_buf();
            }
            _ => {
                break;
            }
        }
    }

    absolute_part.join(relative_part)
}

fn relativise_path<P: AsRef<std::path::Path>, Q: AsRef<std::path::Path>>(
    path: P,
    relative_to: Q,
) -> Option<std::path::PathBuf> {
    let mut path = path.as_ref().to_path_buf();

    if !path.is_absolute() {
        path = std::path::absolute(path).ok()?;
    }

    canonicalise_prefix(path)
        .strip_prefix(canonicalise_prefix(relative_to.as_ref()))
        .ok()
        .map(|x| x.to_path_buf())
}

impl LocalSrcProject {
    pub fn root_path(&self) -> PathBuf {
        self.project_path.clone()
    }

    pub fn info_path(&self) -> PathBuf {
        self.project_path.join(".project.json")
    }

    pub fn meta_path(&self) -> PathBuf {
        self.project_path.join(".meta.json")
    }

    pub fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, LocalSrcError> {
        Ok(self.get_project()?.0)
    }

    pub fn get_meta(&mut self) -> Result<Option<InterchangeProjectMetadataRaw>, LocalSrcError> {
        Ok(self.get_project()?.1)
    }

    pub fn get_unix_path<P: AsRef<std::path::Path>>(
        &self,
        path: P,
    ) -> Result<Utf8UnixPathBuf, UnixPathError> {
        let project_path = self.root_path().canonicalize().map_err(UnixPathError::Io)?;

        let path = relativise_path(&path, project_path).ok_or(
            UnixPathError::PathOutsideProject(path.as_ref().to_path_buf()),
        )?;

        let mut unix_path = Utf8UnixPathBuf::new();
        for component in path.components() {
            unix_path.push(
                component
                    .as_os_str()
                    .to_str()
                    .ok_or(UnixPathError::Conversion)?,
            );
        }

        Ok(unix_path)
    }

    pub fn get_source_path<P: AsRef<Utf8UnixPath>>(&self, path: P) -> Result<PathBuf, PathError> {
        let utf_path = if path.as_ref().is_absolute() {
            if !cfg!(feature = "lenient_checks") {
                return Err(PathError::AbsolutePath(path.as_ref().to_owned()));
            }
            // This should never fail, as the only way for a Unix path to be absolute it to begin
            // at root /.
            path.as_ref()
                .strip_prefix("/")
                .expect("Internal Path processing error")
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
    pub fn get_source_paths(&self) -> Result<HashSet<PathBuf>, LocalSrcError> {
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

    pub fn temporary_from_project<Pr: ProjectRead>(
        project: &Pr,
    ) -> Result<(tempfile::TempDir, Self), CloneError<Pr::Error, LocalSrcError>> {
        let tmp = tempdir()?;
        let mut tmp_project = Self {
            project_path: tmp.path().canonicalize()?,
        };

        clone_project(project, &mut tmp_project, true)?;

        Ok((tmp, tmp_project))
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
                "'.project.json' already exists".to_string(),
            ));
        }

        serde_json::to_writer_pretty(std::fs::File::create(project_json_path)?, info)?;

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

        serde_json::to_writer_pretty(std::fs::File::create(meta_json_path)?, meta)?;

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
                "{} already exists",
                source_path.display()
            )));
        }

        if let Some(parents) = source_path.parent() {
            std::fs::create_dir_all(parents)?;
        }

        std::io::copy(source, &mut std::fs::File::create(source_path)?)?;

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum LocalSrcError {
    #[error("{0}")]
    AlreadyExists(String),
    #[error("project deserialisation error")]
    Serde(#[from] serde_json::Error),
    #[error("project read error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Path(#[from] PathError),
}

#[derive(Error, Debug)]
pub enum UnixPathError {
    #[error("path outside of project: {0}")]
    PathOutsideProject(std::path::PathBuf),
    #[error("io error: {0}")]
    Io(std::io::Error),
    #[error("conversion error")]
    Conversion,
}

#[derive(Error, Debug)]
pub enum PathError {
    #[error("invalid (native) path error")]
    InvalidNativePath(std::path::PathBuf),
    #[error("invalid (encoded) path error")]
    InvalidEncodedPath(typed_path::TypedPathBuf),
    #[error("unsafe path error")]
    UnsafePath(#[from] typed_path::CheckedPathError),
    #[error("absolute path error")]
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
            Some(serde_json::from_reader(std::fs::File::open(
                info_json_path,
            )?)?)
        } else {
            None
        };

        let meta_json_path = self.meta_path();

        let meta_json = if meta_json_path.exists() {
            Some(serde_json::from_reader(std::fs::File::open(
                meta_json_path,
            )?)?)
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

        let f = std::fs::File::open(source_path)?;

        Ok(f)
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self.project_path.to_str() {
            Some(path_str) => vec![crate::lock::Source::LocalSrc {
                src_path: path_str.to_string(),
            }],
            None => vec![],
        }
    }
}
