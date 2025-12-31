// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{self, ProjectRead, editable::GetPath, utils::ZipArchiveError},
};
use std::io::Write as _;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use sha2::Digest as _;
use typed_path::{Utf8Component, Utf8UnixPath};

use thiserror::Error;
use zip::ZipArchive;

use super::utils::{FsIoError, ProjectDeserializationError, ToPathBuf, wrapfs};

/// Project stored in as a KPar (Zip) archive in the local filesystem.
/// Source file paths are interpreted relative to `root`. Both `.project.json`
/// and `.meta.json` are searched for in `root`. If `root` is not given, it is
/// guessed based on the location of the `.project.json`-file.
///
/// Paths used in the archive are expected to match those used in the metadata
/// manifest (.meta.json)! Sysand *MAY* try to normalise paths in order
/// to match filenames, but no guarantees are made.
///
/// Use `LocalKParProject::new_guess_root` to guess `root` based on the
/// presence of a (presumed unique) `.project.json`.
///
/// The archive is read directly without extracting it.
#[derive(Debug)]
pub struct LocalKParProject {
    pub tmp_dir: Utf8TempDir,
    pub archive_path: Utf8PathBuf,
    pub root: Option<Utf8PathBuf>,
}

#[derive(Error, Debug)]
pub enum LocalKParError {
    #[error(transparent)]
    Zip(#[from] project::utils::ZipArchiveError),
    #[error("path `{0}` not found")]
    NotFound(Box<Utf8Path>),
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl From<FsIoError> for LocalKParError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

fn guess_root(archive: &mut ZipArchive<std::fs::File>) -> Result<Utf8PathBuf, LocalKParError> {
    let mut maybe_root = None;
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(ZipArchiveError::FileMeta)?;

        if let Some(p) = file.enclosed_name() {
            // `enclosed_name()` creates path from `String`
            let p = Utf8PathBuf::from_path_buf(p).unwrap();
            if p.file_name() == Some(".project.json") {
                maybe_root = Some(
                    p.parent()
                        .ok_or_else(|| ZipArchiveError::InvalidPath(p.as_path().into()))?
                        .to_path_buf(),
                );
                break;
            }
        }
    }

    if let Some(root) = maybe_root {
        Ok(root)
    } else {
        Err(LocalKParError::NotFound(".project.json".into()))
    }
}

// Wrapping this in case we want to add more normalisation logic
fn path_index<P: AsRef<Utf8UnixPath>, Q: AsRef<Utf8Path>>(
    set_root: Option<Q>,
    archive: &mut ZipArchive<std::fs::File>,
    path: P,
) -> Result<usize, LocalKParError> {
    // NOTE:
    let mut native_path = match set_root {
        Some(root) => root.as_ref().to_path_buf(),
        None => guess_root(archive)?,
    };

    // TODO: Extract this somewhere and clarify behaviour, see
    //       sysand-core/src/io/local_file.rs#L57-78
    //       @ 04c7d46fe2e188602df620407d6cedfef3440eb8
    for component in path.as_ref().components() {
        native_path.push(component.as_str());
    }

    let idx = archive
        .index_for_path(&native_path)
        .ok_or_else(|| LocalKParError::NotFound(native_path.as_path().into()))?;

    Ok(idx)
}

#[derive(Debug, Error)]
pub enum IntoKparError<ReadError> {
    #[error("missing project information file `.project.json`")]
    MissingInfo,
    #[error("missing project metadata file `.meta.json`")]
    MissingMeta,
    #[error(transparent)]
    ProjectRead(ReadError),
    #[error(transparent)]
    Zip(#[from] ZipArchiveError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("project serialization error: {0}: {1}")]
    Serialize(&'static str, serde_json::Error),
}

impl<ReadError> From<FsIoError> for IntoKparError<ReadError> {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl LocalKParProject {
    pub fn new<P: AsRef<Utf8Path>, Q: AsRef<Utf8Path>>(
        path: P,
        root: Q,
    ) -> Result<Self, Box<FsIoError>> {
        Ok(LocalKParProject {
            tmp_dir: tempdir().map_err(FsIoError::MkTempDir)?,
            archive_path: path.as_ref().to_path_buf(),
            root: Some(root.as_ref().to_path_buf()),
        })
    }

    pub fn new_guess_root<P: AsRef<Utf8Path>>(path: P) -> Result<Self, Box<FsIoError>> {
        Ok(LocalKParProject {
            tmp_dir: tempdir().map_err(FsIoError::MkTempDir)?,
            archive_path: path.as_ref().to_path_buf(),
            root: None,
        })
    }

    pub fn from_project<Pr: ProjectRead, P: AsRef<Utf8Path>>(
        from: &Pr,
        path: P,
    ) -> Result<Self, IntoKparError<Pr::Error>> {
        let file = wrapfs::File::create(&path)?;
        let mut zip = zip::ZipWriter::new(file);

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        let (info, meta) = from.get_project().map_err(IntoKparError::ProjectRead)?;
        let info = info.ok_or(IntoKparError::MissingInfo)?;
        let meta = meta.ok_or(IntoKparError::MissingMeta)?;
        let info_content = serde_json::to_string(&info)
            .map_err(|e| IntoKparError::Serialize("failed to serialize project info", e))?;
        let meta_content = serde_json::to_string(&meta)
            .map_err(|e| IntoKparError::Serialize("failed to serialize project metadata", e))?;

        // KerML Clause 10.3: “In addition, the archive shall contain, at its
        // top level, exactly one file named .project.json and exactly one file
        // named .meta.json.”

        zip.start_file(".project.json", options)
            .map_err(|e| ZipArchiveError::Write(Utf8Path::new(".project.json").into(), e))?;
        zip.write(info_content.as_bytes())
            .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;

        zip.start_file(".meta.json", options)
            .map_err(|e| ZipArchiveError::Write(Utf8Path::new(".meta.json").into(), e))?;
        zip.write(meta_content.as_bytes())
            .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;

        for source_path in meta.source_paths(true) {
            let mut reader = from
                .read_source(&source_path)
                .map_err(IntoKparError::ProjectRead)?;
            zip.start_file(&source_path, options)
                .map_err(|e| ZipArchiveError::Write(Utf8Path::new(&source_path).into(), e))?;
            std::io::copy(&mut reader, &mut zip)
                .map_err(|e| FsIoError::CopyFile(source_path.into(), path.to_std_path_buf(), e))?;
        }

        zip.finish()
            .map_err(|e| ZipArchiveError::Finish(path.as_ref().into(), e))?;

        LocalKParProject::new(&path, ".").map_err(IntoKparError::Io)
    }

    fn new_file(&self) -> Result<std::fs::File, LocalKParError> {
        Ok(wrapfs::File::open(&self.archive_path)?)
    }

    fn new_archive(&self) -> Result<ZipArchive<std::fs::File>, LocalKParError> {
        Ok(zip::ZipArchive::new(self.new_file()?)
            .map_err(|e| ZipArchiveError::ReadArchive(self.archive_path.as_path().into(), e))?)
    }

    pub fn file_size(&self) -> Result<u64, LocalKParError> {
        Ok(self
            .new_file()?
            .metadata()
            .map_err(FsIoError::MetadataHandle)?
            .len())
    }
}

impl GetPath for LocalKParProject {
    fn get_path(&self) -> &str {
        self.archive_path.as_str()
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
            Ok(idx) => serde_json::from_reader(
                archive
                    .by_index(idx)
                    .map_err(|e| ZipArchiveError::NamedFileMeta(".project.json".into(), e))?,
            )
            .map_err(|e| {
                ProjectDeserializationError::new("failed to deserialize `.project.json`", e)
            })?,
            Err(LocalKParError::NotFound(_)) => None,
            Err(err) => return Err(err),
        };

        let meta = match path_index(self.root.as_deref(), &mut archive, ".meta.json") {
            Ok(idx) => serde_json::from_reader(
                archive
                    .by_index(idx)
                    .map_err(|e| ZipArchiveError::NamedFileMeta(".meta.json".into(), e))?,
            )
            .map_err(|e| {
                ProjectDeserializationError::new("failed to deserialize `.meta.json`", e)
            })?,
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
        let tmp_file_path = {
            let mut p = self
                .tmp_dir
                .path()
                .canonicalize_utf8()
                .map_err(|e| FsIoError::Canonicalize(self.tmp_dir.to_std_path_buf(), e))?;
            p.push(tmp_name);
            p
        };

        if !tmp_file_path.is_file() {
            let mut tmp_file = wrapfs::File::create(&tmp_file_path)?;

            let mut archive = self.new_archive()?;
            let idx = path_index(self.root.as_deref(), &mut archive, &path)?;

            let mut zip_file = archive
                .by_index(idx)
                .map_err(|e| ZipArchiveError::NamedFileMeta(path.as_ref().as_str().into(), e))?;

            std::io::copy(&mut zip_file, &mut tmp_file)
                .map_err(|e| FsIoError::WriteFile(tmp_file_path.clone(), e))?;
        }

        Ok(super::utils::FileWithLifetime::new(wrapfs::File::open(
            tmp_file_path,
        )?))

        // TODO: Solve this with a ZipFile-handle instead
        // Ok(KparFile { archive: archive, file: &mut archive.by_index(idx)? })
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        vec![crate::lock::Source::LocalKpar {
            kpar_path: self.archive_path.as_str().into(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write};

    use camino_tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    use super::ProjectRead;

    #[test]
    fn test_basic_kpar_archive() -> Result<(), Box<dyn std::error::Error>> {
        let cwd = tempdir()?;
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
        let cwd = tempdir()?;
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
