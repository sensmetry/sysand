// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::PathBuf;

use sha2::Digest;
use tempfile::{TempDir, tempdir};

use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::project::ProjectRead;

/// Project stored at a remote URL such as https://www.example.com/project.kpar.
/// The URL is expected to resolve to a kpar-archive (ZIP-file) (at least) if
/// requested with CONTENT-TYPE(s) application/zip, application/x-zip-compressed.
///
/// See `LocalKParProject` for additional details on the format.
///
/// The archive is accessed using RANGE requests, meaning only the relevant
/// fragment of the archive is downloaded. Otherwise the archive is downloaded
/// lazily, and is deleted at the end of the lifetime of the object.
///
/// For servers that do not support RANGE requests, or if you know the full archive
/// will be consumed, use `HTTPKParProjectDownloaded`.
#[derive(Debug)]
pub struct ReqwestKparRangedProject {
    /// Remote Zip archive
    pub archive: partialzip::PartialZip,
    /// Storage for files (used to provide `Read` access).
    ///
    /// NOTE: This could be replaced by a patch to partialzip, but this
    /// implementation is tentative anyway, as it may require reworking
    /// for async.
    pub tmp_dir: TempDir,
    ///.Root dir of project inside archive
    pub root: Utf8UnixPathBuf,
}

#[derive(Error, Debug)]
pub enum ReqwestKparRangedError {
    #[error(transparent)]
    Zip(#[from] partialzip::PartialZipError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

impl ReqwestKparRangedProject {
    /// Identifies the location of the project inside the archive by locating
    /// the .project.json-file (assuming it is present).
    pub fn new_guess_root<S: AsRef<str>>(url: S) -> Result<Self, ReqwestKparRangedError> {
        let archive = partialzip::PartialZip::new_check_range(&url.as_ref(), true)?;
        let tmp_dir = tempdir()?;

        let mut maybe_root = None;

        for file in archive.list_names() {
            let file_path = Utf8UnixPath::new(&file);

            if file_path.file_name() == Some(".project.json")
                || file_path.file_name() == Some(".meta.json")
            {
                if let Some(root) = file_path.parent() {
                    maybe_root = Some(root.to_path_buf());
                    break;
                }
            }
        }

        let root = maybe_root.unwrap_or_default();

        Ok(ReqwestKparRangedProject {
            archive,
            tmp_dir,
            root,
        })
    }

    pub fn read_path<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<PathBuf, ReqwestKparRangedError> {
        let real_path = self.root.join(path);

        let name = format!("{:X}", sha2::Sha256::digest(&real_path));
        let file_path = self.tmp_dir.path().join(name);
        let mut file = std::fs::File::create(&file_path)?;

        self.archive
            .download_to_write(real_path.as_str(), &mut file)?;

        Ok(file_path)
    }
}

pub type KParRemoteReader<'a> = super::utils::FileWithLifetime<'a>;

impl ProjectRead for ReqwestKparRangedProject {
    type Error = ReqwestKparRangedError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        let info = match self.read_path(".project.json") {
            Ok(info_file) => Some(serde_json::from_reader(std::fs::File::open(info_file)?)?),
            Err(ReqwestKparRangedError::Zip(partialzip::PartialZipError::FileNotFound)) => None,
            Err(err) => {
                return Err(err);
            }
        };

        let meta = match self.read_path(".meta.json") {
            Ok(meta_file) => Some(serde_json::from_reader(std::fs::File::open(meta_file)?)?),
            Err(ReqwestKparRangedError::Zip(partialzip::PartialZipError::FileNotFound)) => None,
            Err(err) => {
                return Err(err);
            }
        };

        Ok((info, meta))
    }

    type SourceReader<'a>
        = KParRemoteReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        Ok(super::utils::FileWithLifetime::new(std::fs::File::open(
            self.read_path(path)?,
        )?))
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        vec![crate::lock::Source::RemoteKpar {
            remote_kpar: self.archive.url(),
            remote_kpar_size: Some(self.archive.file_size()),
        }]
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use std::{
        io::{Read, Write as _},
        process::Command,
        thread::sleep,
        time::Duration,
    };

    use tempfile::tempdir;

    use crate::project::ProjectRead;

    // NOTE: Testing done in manually, due to lack of range header support in all
    //       easy-to-integrate-in-tests HTTP servers.
    #[cfg(feature = "alltests")]
    #[test]
    fn test_basic_range_request() -> Result<(), Box<dyn std::error::Error>> {
        let cwd = tempdir()?;

        let _buf = {
            //let mut cursor = std::io::Cursor::new(vec![]);
            //let mut zip = zip::ZipWriter::new(&mut cursor);

            let file_path = cwd.path().join("project.kpar");
            let file = std::fs::File::create(&file_path)?;
            let mut zip = zip::ZipWriter::new(file);

            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);

            zip.start_file("some_root_dir/.project.json", options)?;
            zip.write_all(br#"{"name":"test_basic_range_request","version":"1.2.3","usage":[]}"#)?;
            zip.start_file("some_root_dir/.meta.json", options)?;
            zip.write_all(br#"{"index":{},"created":"123"}"#)?;
            zip.start_file("some_root_dir/test.sysml", options)?;
            zip.write_all(br#"package Test;"#)?;

            zip.finish().unwrap();

            //cursor.flush()?;
            //cursor.into_inner()
            file_path
        };

        let free_port = port_check::free_local_port().unwrap().to_string();

        let mut server = Command::new("uv")
            .arg("run")
            .arg("--with")
            .arg("rangehttpserver")
            .arg("-m")
            .arg("RangeHTTPServer")
            .arg(&free_port)
            .current_dir(cwd.path())
            .spawn()?;
        sleep(Duration::from_millis(1000));

        let project = super::ReqwestKparRangedProject::new_guess_root(format!(
            "http://localhost:{}/project.kpar",
            &free_port
        ))?;

        assert_eq!(project.root, "some_root_dir/");

        let (Some(info), Some(meta)) = project.get_project()? else {
            panic!()
        };

        assert_eq!(info.name, "test_basic_range_request");
        assert_eq!(meta.created, "123");

        let mut src = "".to_string();
        project
            .read_source("test.sysml")?
            .read_to_string(&mut src)?;

        assert_eq!(src, "package Test;");

        //get_kpar.assert();

        server.kill()?;

        Ok(())
    }
}
