use fluent_uri::component::Scheme;
use thiserror::Error;

use crate::{
    project::gix_git_download::{GixDownloadedError, GixDownloadedProject},
    resolve::{
        ResolveRead,
        file::SCHEME_FILE,
        reqwest_http::{SCHEME_HTTP, SCHEME_HTTPS},
    },
};

#[derive(Debug)]
pub struct GitResolver {}

#[derive(Error, Debug)]
pub enum GitResolverError {
    #[error("{0}")]
    GitProjectError(#[from] GixDownloadedError),
}

pub const SCHEME_SSH: &Scheme = Scheme::new_or_panic("ssh");

impl ResolveRead for GitResolver {
    type Error = GitResolverError;

    type ProjectStorage = GixDownloadedProject;

    type ResolvedStorages = std::iter::Once<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let scheme = uri.scheme();

        if !(scheme == SCHEME_HTTP
            || scheme == SCHEME_HTTPS
            || scheme == SCHEME_FILE
            || scheme == SCHEME_SSH)
        {
            return Ok(crate::resolve::ResolutionOutcome::UnsupportedIRIType(
                format!("not a known git-compatible url scheme {}", uri.as_str()),
            ));
        }

        Ok(crate::resolve::ResolutionOutcome::Resolved(
            std::iter::once(GixDownloadedProject::new(uri.as_str()).map_err(|e| e.into())),
        ))
    }

    fn resolve_read_raw<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        if let Some(stripped_uri) = uri.as_ref().strip_prefix("git+") {
            self.default_resolve_read_raw(stripped_uri)
        } else {
            self.default_resolve_read_raw(uri)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::resolve::{ResolutionOutcome, ResolveRead, gix_git::GitResolver};

    fn un_once<T>(x: &mut std::iter::Once<T>) -> T {
        x.next().unwrap()
    }

    #[test]
    fn basic_url_examples() -> Result<(), Box<dyn std::error::Error>> {
        let resolver = GitResolver {};

        let ResolutionOutcome::Resolved(mut one_http_proj) =
            resolver.resolve_read_raw("http://www.example.com/proj")?
        else {
            panic!("expected http url to resolve");
        };
        assert_eq!(
            un_once(&mut one_http_proj).unwrap().url.to_string(),
            "http://www.example.com/proj"
        );

        let ResolutionOutcome::Resolved(mut one_https_proj) =
            resolver.resolve_read_raw("https://www.example.com/proj")?
        else {
            panic!("expected https url to resolve");
        };
        assert_eq!(
            un_once(&mut one_https_proj).unwrap().url.to_string(),
            "https://www.example.com/proj"
        );

        let ResolutionOutcome::Resolved(mut one_ssh_proj) =
            resolver.resolve_read_raw("ssh://www.example.com/proj")?
        else {
            panic!("expected ssh url to resolve");
        };
        assert_eq!(
            un_once(&mut one_ssh_proj).unwrap().url.to_string(),
            "ssh://www.example.com/proj"
        );

        let ResolutionOutcome::Resolved(mut one_file_proj) =
            resolver.resolve_read_raw("file://www.example.com/proj")?
        else {
            panic!("expected file url to resolve");
        };
        assert_eq!(
            un_once(&mut one_file_proj).unwrap().url.to_string(),
            "file://www.example.com/proj"
        );

        let ResolutionOutcome::Resolved(mut one_git_http_proj) =
            resolver.resolve_read_raw("git+http://www.example.com/proj")?
        else {
            panic!("expected git+http url to resolve");
        };
        assert_eq!(
            un_once(&mut one_git_http_proj).unwrap().url.to_string(),
            "http://www.example.com/proj"
        );

        let ResolutionOutcome::Resolved(mut one_git_https_proj) =
            resolver.resolve_read_raw("git+https://www.example.com/proj")?
        else {
            panic!("expected git+https url to resolve");
        };
        assert_eq!(
            un_once(&mut one_git_https_proj).unwrap().url.to_string(),
            "https://www.example.com/proj"
        );

        let ResolutionOutcome::Resolved(mut one_git_ssh_proj) =
            resolver.resolve_read_raw("git+ssh://www.example.com/proj")?
        else {
            panic!("expected git+ssh url to resolve");
        };
        assert_eq!(
            un_once(&mut one_git_ssh_proj).unwrap().url.to_string(),
            "ssh://www.example.com/proj"
        );

        let ResolutionOutcome::Resolved(mut one_git_file_proj) =
            resolver.resolve_read_raw("git+file://www.example.com/proj")?
        else {
            panic!("expected git+file url to resolve");
        };
        assert_eq!(
            un_once(&mut one_git_file_proj).unwrap().url.to_string(),
            "file://www.example.com/proj"
        );

        Ok(())
    }
}
