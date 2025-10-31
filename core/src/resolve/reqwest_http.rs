// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{convert::Infallible, io::Read};

use fluent_uri::component::Scheme;
use thiserror::Error;

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectRead, reqwest_kpar_download::ReqwestKparDownloadedProject,
        reqwest_kpar_ranged::ReqwestKparRangedProject, reqwest_src::ReqwestSrcProject,
    },
    resolve::ResolveRead,
};

/// Tries to resolve http(s) URLs as direct (resolvable) links to interchange projects.
#[derive(Debug)]
pub struct HTTPResolver {
    pub client: reqwest::blocking::Client,
    pub lax: bool,
    pub prefer_ranged: bool,
}

pub const SCHEME_HTTP: &Scheme = Scheme::new_or_panic("http");
pub const SCHEME_HTTPS: &Scheme = Scheme::new_or_panic("https");

#[derive(Debug)]
pub enum HTTPProject {
    HTTPSrcProject(ReqwestSrcProject),
    HTTPKParProjectRanged(ReqwestKparRangedProject),
    HTTPKParProjectDownloaded(ReqwestKparDownloadedProject),
}

#[derive(Error, Debug)]
pub enum HTTPProjectError {
    #[error(transparent)]
    SrcProject(<ReqwestSrcProject as ProjectRead>::Error),
    #[error(transparent)]
    KParRanged(<ReqwestKparRangedProject as ProjectRead>::Error),
    #[error(transparent)]
    KparDownloaded(<ReqwestKparDownloadedProject as ProjectRead>::Error),
}

pub enum HTTPProjectReader<'a> {
    SrcProjectReader(<ReqwestSrcProject as ProjectRead>::SourceReader<'a>),
    KParRangedReader(<ReqwestKparRangedProject as ProjectRead>::SourceReader<'a>),
    KparDownloadedReader(<ReqwestKparDownloadedProject as ProjectRead>::SourceReader<'a>),
}

impl Read for HTTPProjectReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            HTTPProjectReader::SrcProjectReader(proj) => proj.read(buf),
            HTTPProjectReader::KParRangedReader(proj) => proj.read(buf),
            HTTPProjectReader::KparDownloadedReader(proj) => proj.read(buf),
        }
    }
}

impl ProjectRead for HTTPProject {
    type Error = HTTPProjectError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        match self {
            HTTPProject::HTTPSrcProject(proj) => {
                proj.get_project().map_err(HTTPProjectError::SrcProject)
            }
            HTTPProject::HTTPKParProjectRanged(proj) => {
                proj.get_project().map_err(HTTPProjectError::KParRanged)
            }
            HTTPProject::HTTPKParProjectDownloaded(proj) => {
                proj.get_project().map_err(HTTPProjectError::KparDownloaded)
            }
        }
    }

    type SourceReader<'a>
        = HTTPProjectReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self {
            HTTPProject::HTTPSrcProject(proj) => proj
                .read_source(path)
                .map_err(HTTPProjectError::SrcProject)
                .map(HTTPProjectReader::SrcProjectReader),
            HTTPProject::HTTPKParProjectRanged(proj) => proj
                .read_source(path)
                .map_err(HTTPProjectError::KParRanged)
                .map(HTTPProjectReader::KParRangedReader),
            HTTPProject::HTTPKParProjectDownloaded(proj) => proj
                .read_source(path)
                .map_err(HTTPProjectError::KparDownloaded)
                .map(HTTPProjectReader::KparDownloadedReader),
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match self {
            HTTPProject::HTTPSrcProject(proj) => proj.is_definitely_invalid(),
            HTTPProject::HTTPKParProjectRanged(proj) => proj.is_definitely_invalid(),
            HTTPProject::HTTPKParProjectDownloaded(proj) => proj.is_definitely_invalid(),
        }
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self {
            HTTPProject::HTTPSrcProject(proj) => proj.sources(),
            HTTPProject::HTTPKParProjectRanged(proj) => proj.sources(),
            HTTPProject::HTTPKParProjectDownloaded(proj) => proj.sources(),
        }
    }
}

pub struct HTTPProjects {
    client: reqwest::blocking::Client,
    url: reqwest::Url,
    src_done: bool,
    kpar_done: bool,
    // See the comments in `try_resolve_as_src`.
    lax: bool,
    prefer_ranged: bool,
}

impl HTTPProjects {
    pub fn try_resolve_as_kpar(&self) -> Option<HTTPProject> {
        // TODO: Decide a policy for KPar vs Src urls
        let url = if self.url.path() == "" || !self.url.path().ends_with("/") {
            self.url.clone()
        // If the resolver is set to be lax, try forcing the terminal slash
        } else if self.lax {
            let mut lax_url = self.url.clone();
            let lax_path = lax_url.path().to_string();
            lax_url.set_path(
                lax_path
                    .strip_suffix('/')
                    .expect("internal url processing error"),
            );

            lax_url
        } else {
            return None;
        };

        if self.prefer_ranged {
            if let Ok(proj) = ReqwestKparRangedProject::new_guess_root(&url) {
                return Some(HTTPProject::HTTPKParProjectRanged(proj));
            }
        }

        Some(HTTPProject::HTTPKParProjectDownloaded(
            ReqwestKparDownloadedProject::new_guess_root(&url).expect("internal IO error"),
        ))
    }

    pub fn try_resolve_as_src(&self) -> Option<HTTPProject> {
        // These URLs should technically have a path that ends (explicitly or implicitly)
        // with a slash, due to the way relative references are treated in HTTP. E.g.:
        // resolving 'bar' relatieve to 'http://www.example.com/foo' gives 'http://www.example.com/bar'
        // while resolving 'bar' relatieve to 'http://www.example.com/foo/' gives 'http://www.example.com/foo/bar'
        if self.url.path() == "" || self.url.path().ends_with("/") {
            Some(HTTPProject::HTTPSrcProject(ReqwestSrcProject {
                client: self.client.clone(), // Already internally an Rc
                url: self.url.clone(),
            }))
        // If the resolver is set to be lax, try forcing the terminal slash
        } else if self.lax {
            let mut lax_url = self.url.clone();
            let mut lax_path = lax_url.path().to_string();
            lax_path.push('/');
            lax_url.set_path(&lax_path);

            Some(HTTPProject::HTTPSrcProject(ReqwestSrcProject {
                client: self.client.clone(), // Already internally an Rc
                url: lax_url,
            }))
        } else {
            None
        }
    }
}

impl Iterator for HTTPProjects {
    type Item = Result<HTTPProject, Infallible>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.src_done {
            self.src_done = true;

            if let Some(proj) = self.try_resolve_as_src() {
                return Some(Ok(proj));
            }
        }

        if !self.kpar_done {
            self.kpar_done = true;

            if let Some(proj) = self.try_resolve_as_kpar() {
                return Some(Ok(proj));
            }
        }

        None
    }
}

/// Tries treat IRIs as HTTP URLs, pointing either to source files stored remotely
/// or a KPar archive stored remotely.
///
/// If `prefer_ranged` is true, it attempts to poke the remote server to see if it
/// appears to support HTTP Range requests. If successful, it uses `HTTPKparProjectRanged`
/// instead of `HTTPKparProjectDownloaded`. In case of *any* failure, or if `prefer_ranged`
/// is false, `HTTPKparProjectDownloaded` is used instead.
impl ResolveRead for HTTPResolver {
    type Error = Infallible;

    type ProjectStorage = HTTPProject;

    type ResolvedStorages = HTTPProjects;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        // Try to resolve as a HTTP src project.
        Ok(
            if uri.scheme() == SCHEME_HTTP || uri.scheme() == SCHEME_HTTPS {
                if let Ok(url) = reqwest::Url::parse(uri.as_str()) {
                    super::ResolutionOutcome::Resolved(HTTPProjects {
                        client: self.client.clone(),
                        url,
                        src_done: false,
                        kpar_done: false,
                        lax: self.lax,
                        prefer_ranged: self.prefer_ranged,
                    })
                } else {
                    super::ResolutionOutcome::UnsupportedIRIType("invalid http(s) URL".to_string())
                }
            } else {
                super::ResolutionOutcome::UnsupportedIRIType("not an http(s) URL".to_string())
            },
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]

    use std::{io::Write as _, process::Command, thread::sleep, time::Duration};

    use crate::{
        project::ProjectRead,
        resolve::{ResolutionOutcome, ResolveRead},
    };

    #[test]
    fn test_basic_http_src_url_non_lax() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let host = server.host_with_port();

        let info_mock = server
            .mock("GET", "/foo/.project.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"name":"test_basic_http_src_url","version":"1.2.3","usage":[]}"#)
            .create();

        let meta_mock = server
            .mock("GET", "/foo/.meta.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
            .create();

        let client = reqwest::blocking::ClientBuilder::new().build().unwrap();

        let resolver = super::HTTPResolver {
            client,
            lax: false,
            prefer_ranged: true,
        };

        let ResolutionOutcome::Resolved(projects) =
            resolver.resolve_read_raw(format!("http://{}/foo/", host))?
        else {
            panic!()
        };

        for project in projects {
            let project = project?;

            let (Some(info), Some(meta)) = project.get_project()? else {
                panic!()
            };

            assert_eq!(info.name, "test_basic_http_src_url");
            assert_eq!(meta.created, "0000-00-00T00:00:00.123456789Z");
        }

        info_mock.assert();
        meta_mock.assert();

        Ok(())
    }

    fn template_basic_http_url_lax(
        with_slash: bool,
        prefer_ranged: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = reqwest::blocking::ClientBuilder::new().build().unwrap();

        let resolver = super::HTTPResolver {
            client,
            lax: true,
            prefer_ranged,
        };

        let url = if with_slash {
            "http://www.example.invalid/foo/"
        } else {
            "http://www.example.invalid/foo"
        };

        let ResolutionOutcome::Resolved(projects) = resolver.resolve_read_raw(url)? else {
            panic!()
        };
        let projects: Vec<super::HTTPProject> = projects.into_iter().map(|x| x.unwrap()).collect();

        assert_eq!(projects.len(), 2);

        let mut found_src = false;
        let mut found_kpar = false;

        for project in projects {
            match project {
                crate::resolve::reqwest_http::HTTPProject::HTTPSrcProject(_) => {
                    found_src = true;
                }
                crate::resolve::reqwest_http::HTTPProject::HTTPKParProjectRanged(_) => {
                    panic!("got a ranged project for invalid url");
                }
                crate::resolve::reqwest_http::HTTPProject::HTTPKParProjectDownloaded(_) => {
                    found_kpar = true;
                }
            }
        }

        assert!(found_kpar);
        assert!(found_src);

        Ok(())
    }

    #[test]
    fn test_basic_http_url_lax_with_slash_prefer_ranged() -> Result<(), Box<dyn std::error::Error>>
    {
        template_basic_http_url_lax(true, true)
    }

    #[test]
    fn test_basic_http_url_lax_with_slash_not_prefer_ranged()
    -> Result<(), Box<dyn std::error::Error>> {
        template_basic_http_url_lax(true, false)
    }

    #[test]
    fn test_basic_http_url_lax_without_slash_prefer_ranged()
    -> Result<(), Box<dyn std::error::Error>> {
        template_basic_http_url_lax(false, true)
    }

    #[test]
    fn test_basic_http_url_lax_without_slash_not_prefer_ranged()
    -> Result<(), Box<dyn std::error::Error>> {
        template_basic_http_url_lax(false, false)
    }

    // NOTE: Testing done in manually, due to lack of range header support in all
    //       easy-to-integrate-in-tests HTTP servers.
    #[cfg(feature = "alltests")]
    #[test]
    fn test_resolves_ranged_if_successful() -> Result<(), Box<dyn std::error::Error>> {
        let cwd = tempfile::tempdir()?;

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
            zip.write_all(
                br#"{"name":"test_resolves_ranged_if_successful","version":"1.2.3","usage":[]}"#,
            )?;
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

        let client = reqwest::blocking::ClientBuilder::new().build().unwrap();
        let resolver = super::HTTPResolver {
            client,
            lax: false,
            prefer_ranged: true,
        };

        let ResolutionOutcome::Resolved(projects) =
            resolver.resolve_read_raw(format!("http://localhost:{}/project.kpar", &free_port))?
        else {
            panic!()
        };

        let projects: Vec<super::HTTPProject> = projects.into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(projects.len(), 1);

        if let crate::resolve::reqwest_http::HTTPProject::HTTPKParProjectRanged(_) = projects[0] {
        } else {
            panic!("expected a ranged project");
        }

        server.kill()?;

        Ok(())
    }

    #[test]
    fn test_resolves_non_ranged_if_unsupported() -> Result<(), Box<dyn std::error::Error>> {
        let buf = {
            let mut cursor = std::io::Cursor::new(vec![]);
            let mut zip = zip::ZipWriter::new(&mut cursor);

            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);

            zip.start_file("some_root_dir/.project.json", options)?;
            zip.write_all(
                br#"{"name":"test_resolves_non_ranged_if_unsupported","version":"1.2.3","usage":[]}"#,
            )?;
            zip.start_file("some_root_dir/.meta.json", options)?;
            zip.write_all(br#"{"index":{},"created":"123"}"#)?;
            zip.start_file("some_root_dir/test.sysml", options)?;
            zip.write_all(br#"package Test;"#)?;

            zip.finish().unwrap();

            cursor.flush()?;
            cursor.into_inner()
        };

        let mut server = mockito::Server::new();

        //let host = server.host_with_port();
        let url = reqwest::Url::parse(&server.url()).unwrap();

        // Should only generate a HEAD request
        let get_kpar = server
            .mock("HEAD", "/project.kpar")
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body(&buf)
            .create();

        let client = reqwest::blocking::ClientBuilder::new().build().unwrap();
        let resolver = super::HTTPResolver {
            client,
            lax: false,
            prefer_ranged: true,
        };

        let ResolutionOutcome::Resolved(projects) =
            resolver.resolve_read_raw(format!("{}project.kpar", url,))?
        else {
            panic!()
        };

        let projects: Vec<super::HTTPProject> = projects.into_iter().map(|x| x.unwrap()).collect();
        assert_eq!(projects.len(), 1);

        if let crate::resolve::reqwest_http::HTTPProject::HTTPKParProjectDownloaded(_) = projects[0]
        {
        } else {
            panic!("expected a ranged project");
        }

        get_kpar.assert();

        Ok(())
    }
}
