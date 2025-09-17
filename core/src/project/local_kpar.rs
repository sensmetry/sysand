// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::ProjectRead,
};
use std::{
    io::Write as _,
    path::{Path, PathBuf},
};

use sha2::Digest as _;
use tempfile::{TempDir, tempdir};
use typed_path::{Utf8Component, Utf8UnixPath};

use thiserror::Error;
use zip::ZipArchive;

/// Project stored in as a KPar (Zip) archive in the local filesystem.
/// Source file paths are interpreted relative to `root`. Both `.project.json`
/// and `.meta.json` are searched for in `root`. If `root` is not given, it is
/// guessed based on the location of the `.project.json`-file.
///
/// Paths used in the archive are expected to match those used in the metadata
/// manifest (.meta.json)! Sysand *MAY* try to normalise paths in order
/// to match filenames, but no guarnatees are made.
///
/// Use `LocalKParProject::new_guess_root` to guess `root` based on the
/// presence of a (presumed unique) `.project.json`.
///
/// The archive is read directly without extracting it.
#[derive(Debug)]
pub struct LocalKParProject {
    pub tmp_dir: TempDir,
    pub archive_path: PathBuf,
    pub root: Option<PathBuf>,
}

#[derive(Error, Debug)]
pub enum LocalKParError {
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid name in archive: {0}")]
    InvalidName(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn guess_root(archive: &mut ZipArchive<std::fs::File>) -> Result<PathBuf, LocalKParError> {
    let mut maybe_root = None;
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;

        if let Some(p) = file.enclosed_name() {
            if p.file_name() == Some(std::ffi::OsStr::new(".project.json")) {
                let err_msg = format!(
                    "internal path handling error: path {} is not contained in a directory",
                    p.display()
                );

                maybe_root = Some(p.parent().expect(&err_msg).to_path_buf());
                break;
            }
        }
    }

    if let Some(root) = maybe_root {
        Ok(root)
    } else {
        Err(LocalKParError::NotFound(".project.json".to_string()))
    }
}

// Wrapping this in case we want to add more normalisation logic
fn path_index<P: AsRef<Utf8UnixPath>>(
    set_root: Option<&Path>,
    archive: &mut ZipArchive<std::fs::File>,
    path: P,
) -> Result<usize, LocalKParError> {
    // NOTE:
    let mut native_path = match set_root {
        Some(root) => root.to_path_buf(),
        None => guess_root(archive)?,
    };

    // TODO: Extract this somewhere and clarify behaviour, see
    //       sysand-core/src/io/local_file.rs#L57-78
    //       @ 04c7d46fe2e188602df620407d6cedfef3440eb8
    for component in path.as_ref().components() {
        native_path.push(component.as_str());
    }

    let idx = archive.index_for_path(&native_path).ok_or_else(|| {
        LocalKParError::NotFound(native_path.as_os_str().to_string_lossy().to_string())
    })?;

    Ok(idx)
}

#[derive(Debug, Error)]
pub enum IntoKparError<ReadError> {
    #[error("missing project information")]
    MissingInfo,
    #[error("missing project metadata")]
    MissingMeta,
    #[error("{0}")]
    ReadError(ReadError),
    #[error("{0}")]
    ZipWriteError(zip::result::ZipError),
    #[error("failed to use path {0}")]
    PathFailure(String),
    #[error("{0}")]
    IOError(std::io::Error),
    #[error("file name error")]
    FileNameError,
    #[error("serde error: {0}")]
    SerdeError(serde_json::Error),
}

impl LocalKParProject {
    pub fn new<P: AsRef<Path>, Q: AsRef<Path>>(path: P, root: Q) -> Result<Self, std::io::Error> {
        Ok(LocalKParProject {
            tmp_dir: tempdir()?,
            archive_path: path.as_ref().to_path_buf(),
            root: Some(root.as_ref().to_path_buf()),
        })
    }

    pub fn new_guess_root<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        Ok(LocalKParProject {
            tmp_dir: tempdir()?,
            archive_path: path.as_ref().to_path_buf(),
            root: None,
        })
    }

    pub fn from_project<Pr: ProjectRead, P: AsRef<Path>>(
        from: &Pr,
        path: P,
    ) -> Result<Self, IntoKparError<Pr::Error>> {
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        let (info, meta) = from.get_project().map_err(IntoKparError::ReadError)?;
        let info = info.ok_or(IntoKparError::MissingInfo)?;
        let meta = meta.ok_or(IntoKparError::MissingMeta)?;
        let info_content = serde_json::to_string(&info).map_err(IntoKparError::SerdeError)?;
        let meta_content = serde_json::to_string(&meta).map_err(IntoKparError::SerdeError)?;

        // KerML Clause 10.3: “In addition, the archive shall contain, at its
        // top level, exactly one file named .project.json and exactly one file
        // named .meta.json.”

        zip.start_file(".project.json", options)
            .map_err(IntoKparError::ZipWriteError)?;
        zip.write(info_content.as_bytes())
            .map_err(IntoKparError::IOError)?;

        zip.start_file(".meta.json", options)
            .map_err(IntoKparError::ZipWriteError)?;
        zip.write(meta_content.as_bytes())
            .map_err(IntoKparError::IOError)?;

        for source_path in meta.source_paths(true) {
            let mut reader = from
                .read_source(&source_path)
                .map_err(IntoKparError::ReadError)?;
            zip.start_file(source_path, options)
                .map_err(IntoKparError::ZipWriteError)?;
            std::io::copy(&mut reader, &mut zip).map_err(IntoKparError::IOError)?;
        }

        zip.finish().map_err(IntoKparError::ZipWriteError)?;

        LocalKParProject::new(&path, ".").map_err(IntoKparError::IOError)
    }

    fn new_file(&self) -> Result<std::fs::File, LocalKParError> {
        Ok(std::fs::File::open(&self.archive_path)?)
    }

    fn new_archive(&self) -> Result<ZipArchive<std::fs::File>, LocalKParError> {
        Ok(zip::ZipArchive::new(self.new_file()?)?)
    }

    pub fn file_size(&self) -> Result<u64, LocalKParError> {
        Ok(self.new_file()?.metadata()?.len())
    }
}

type KParFile<'a> = super::utils::FileWithLifetime<'a>;

// NOTE: Current implementation keeps re-opening the archive file. This appears to
//       be unavoidable with the current design of this trait.
impl ProjectRead for LocalKParProject {
    type Error = LocalKParError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        let mut archive = self.new_archive()?;

        let info = match path_index(self.root.as_deref(), &mut archive, ".project.json") {
            Ok(idx) => serde_json::from_reader(archive.by_index(idx)?)?,
            Err(LocalKParError::NotFound(_)) => None,
            Err(err) => return Err(err),
        };

        let meta = match path_index(self.root.as_deref(), &mut archive, ".meta.json") {
            Ok(idx) => serde_json::from_reader(archive.by_index(idx)?)?,
            Err(LocalKParError::NotFound(_)) => None,
            Err(err) => return Err(err),
        };

        Ok((info, meta))
    }

    type SourceReader<'a>
        = KParFile<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        let tmp_name = format!("{:X}", sha2::Sha256::digest(path.as_ref()));
        let tmp_file_path = self.tmp_dir.path().canonicalize()?.join(tmp_name);

        if !tmp_file_path.is_file() {
            let mut tmp_file = std::fs::File::create(&tmp_file_path)?;

            let mut archive = self.new_archive()?;
            let idx = path_index(self.root.as_deref(), &mut archive, path)?;

            let mut zip_file = archive.by_index(idx)?;

            std::io::copy(&mut zip_file, &mut tmp_file)?;
        }

        Ok(super::utils::FileWithLifetime::new(std::fs::File::open(
            tmp_file_path,
        )?))

        // TODO: Solve this with a ZipFile-handle instead
        // Ok(KparFile { archive: archive, file: &mut archive.by_index(idx)? })
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self.archive_path.to_str() {
            Some(path_str) => vec![crate::lock::Source::LocalKpar {
                kpar_path: path_str.to_string(),
            }],
            None => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write};

    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    use super::ProjectRead;

    #[test]
    fn test_basic_kpar_archive() -> Result<(), Box<dyn std::error::Error>> {
        let cwd = TempDir::new()?;
        let zip_path = cwd.path().join("test.kpar");

        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);

            zip.start_file(".project.json", options)?;
            zip.write_all(br#"{"name":"test_basic_kpar_archive","version":"1.2.3","usage":[]}"#)?;
            zip.start_file(".meta.json", options)?;
            zip.write_all(br#"{"index":{},"created":"123"}"#)?;
            zip.start_file("test.sysml", options)?;
            zip.write_all(br#"package Test;"#)?;

            zip.finish().unwrap();
        }

        let project = super::LocalKParProject::new_guess_root(zip_path)?;

        let (Some(info), Some(meta)) = project.get_project()? else {
            panic!();
        };

        assert_eq!(info.name, "test_basic_kpar_archive");
        assert_eq!(info.version, "1.2.3");
        assert_eq!(meta.created, "123");

        let mut src = "".to_string();
        project
            .read_source("test.sysml")?
            .read_to_string(&mut src)?;

        assert_eq!(src, "package Test;");

        Ok(())
    }

    #[test]
    fn test_nested_kpar_archive() -> Result<(), Box<dyn std::error::Error>> {
        let cwd = TempDir::new()?;
        let zip_path = cwd.path().join("test.kpar");

        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);

            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);

            zip.start_file("some_root_dir/.project.json", options)?;
            zip.write_all(br#"{"name":"test_nested_kpar_archive","version":"1.2.3","usage":[]}"#)?;
            zip.start_file("some_root_dir/.meta.json", options)?;
            zip.write_all(br#"{"index":{},"created":"123"}"#)?;
            zip.start_file("some_root_dir/test.sysml", options)?;
            zip.write_all(br#"package Test;"#)?;

            zip.finish().unwrap();
        }

        let project = super::LocalKParProject::new_guess_root(zip_path)?;

        let (Some(info), Some(meta)) = project.get_project()? else {
            panic!();
        };

        assert_eq!(info.name, "test_nested_kpar_archive");
        assert_eq!(info.version, "1.2.3");
        assert_eq!(meta.created, "123");

        let mut src = "".to_string();
        project
            .read_source("test.sysml")?
            .read_to_string(&mut src)?;

        assert_eq!(src, "package Test;");

        Ok(())
    }
}
