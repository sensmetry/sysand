use std::convert::Infallible;

use camino::Utf8Path;
use fluent_uri::component::Scheme;
use thiserror::Error;

use crate::{
    model::{InterchangeProjectUsage, InterchangeProjectUsageRaw},
    project::gix_git_download::{
        GixDownloadedError, GixDownloadedProject, GixDownloadedProjectExact,
    },
    resolve::{
        ResolutionOutcome, ResolveRead,
        file::SCHEME_FILE,
        reqwest_http::{SCHEME_HTTP, SCHEME_HTTPS},
    },
};

#[derive(Debug)]
pub struct GitResolver {}

#[derive(Error, Debug)]
pub enum GitResolverError {
    #[error(transparent)]
    GitProject(#[from] GixDownloadedError),
}

pub const SCHEME_SSH: &Scheme = Scheme::new_or_panic("ssh");
pub const SCHEME_GIT_SSH: &Scheme = Scheme::new_or_panic("git+ssh");
pub const SCHEME_GIT_FILE: &Scheme = Scheme::new_or_panic("git+file");
pub const SCHEME_GIT_HTTP: &Scheme = Scheme::new_or_panic("git+http");
pub const SCHEME_GIT_HTTPS: &Scheme = Scheme::new_or_panic("git+https");

impl ResolveRead for GitResolver {
    type Error = GitResolverError;

    type ProjectStorage = GixDownloadedProjectExact;

    type ResolvedStorages = std::iter::Once<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        usage: &InterchangeProjectUsage,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        // TODO: should URL usages be supported for git?
        let outcome = match usage {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint,
            } => {
                let scheme = resource.scheme();

                if ![
                    SCHEME_HTTP,
                    SCHEME_HTTPS,
                    SCHEME_FILE,
                    SCHEME_SSH,
                    SCHEME_GIT_HTTP,
                    SCHEME_GIT_HTTPS,
                    SCHEME_GIT_FILE,
                    SCHEME_GIT_SSH,
                ]
                .contains(&scheme)
                {
                    return Ok(ResolutionOutcome::UnsupportedUsageType {
                        usage: usage.to_owned(),
                        reason: format!("url scheme `{scheme}` is not known to be git-compatible"),
                    });
                }

                ResolutionOutcome::Resolved(std::iter::once(
                    // TODO: use trim_prefix() once it's stable
                    GixDownloadedProjectExact::new_download_find(
                        resource
                            .as_str()
                            .strip_prefix("git+")
                            .unwrap_or(resource.as_str()),
                        None,
                        None::<(&str, &str)>,
                    )
                    .map_err(|e| e.into()),
                ))
            }
            InterchangeProjectUsage::Git {
                git: iri,
                id,
                publisher,
                name,
            } => {
                let scheme = iri.scheme();

                if ![
                    SCHEME_HTTP,
                    SCHEME_HTTPS,
                    SCHEME_FILE,
                    SCHEME_SSH,
                    SCHEME_GIT_HTTP,
                    SCHEME_GIT_HTTPS,
                    SCHEME_GIT_FILE,
                    SCHEME_GIT_SSH,
                ]
                .contains(&scheme)
                {
                    return Ok(ResolutionOutcome::UnsupportedUsageType {
                        usage: usage.to_owned(),
                        reason: format!("url scheme `{scheme}` is not known to be git-compatible"),
                    });
                }

                ResolutionOutcome::Resolved(std::iter::once(
                    // TODO: use trim_prefix() once it's stable
                    GixDownloadedProjectExact::new_download_find(
                        iri.as_str().strip_prefix("git+").unwrap_or(iri.as_str()),
                        Some(id),
                        Some((publisher, name)),
                    )
                    .map_err(|e| e.into()),
                ))
            }
            _ => ResolutionOutcome::UnsupportedUsageType {
                usage: usage.to_owned(),
                reason: String::from("not a url/resource usage"),
            },
        };
        Ok(outcome)
    }

    // fn resolve_read_raw(
    //     &self,
    //     usage: &InterchangeProjectUsageRaw,
    //     base_path: Option<impl AsRef<Utf8Path>>,
    // ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
    //     if let Some(stripped_uri) = uri.as_ref().strip_prefix("git+") {
    //         self.default_resolve_read_raw(stripped_uri)
    //     } else {
    //         self.default_resolve_read_raw(uri)
    //     }
    // }
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
