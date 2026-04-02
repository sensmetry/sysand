// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use bytes::Bytes;
use camino::Utf8Path;
use thiserror::Error;
use url::Url;

use crate::{
    auth::{GlobMapResult, HTTPAuthentication, PublishHTTPAuthentication},
    project::{ProjectRead, local_kpar::LocalKParProject},
};

// Publish-only canonicalization rules for modern project IDs.
// If additional surfaces need this behavior, extract to a shared module.
fn is_ascii_alnum(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

fn is_canonicalizable_field_with_allowed_separators(s: &str, allow_dot: bool) -> bool {
    let bytes = s.as_bytes();
    if !(3..=50).contains(&bytes.len()) {
        return false;
    }

    if !is_ascii_alnum(bytes[0]) || !is_ascii_alnum(bytes[bytes.len() - 1]) {
        return false;
    }

    for i in 1..(bytes.len() - 1) {
        let b = bytes[i];
        if is_ascii_alnum(b) {
            continue;
        }

        let is_separator = b == b'-' || b == b' ' || (allow_dot && b == b'.');
        if !is_separator {
            return false;
        }

        if !is_ascii_alnum(bytes[i - 1]) || !is_ascii_alnum(bytes[i + 1]) {
            return false;
        }
    }

    true
}

fn is_canonicalizable_publisher_field_value(s: &str) -> bool {
    is_canonicalizable_field_with_allowed_separators(s, false)
}

fn is_canonicalizable_name_field_value(s: &str) -> bool {
    is_canonicalizable_field_with_allowed_separators(s, true)
}

fn canonicalize_modern_project_id_component(s: &str) -> String {
    s.to_ascii_lowercase().replace(' ', "-")
}

#[derive(Error, Debug)]
pub enum PublishError {
    #[error("failed to read kpar file at `{0}`: {1}")]
    KparRead(Box<str>, std::io::Error),

    #[error("failed to open kpar project at `{0}`: {1}")]
    KparOpen(Box<str>, String),

    #[error("missing project info in kpar")]
    MissingInfo,

    #[error("missing project metadata in kpar")]
    MissingMeta,

    #[error("missing publisher in project info (required for publishing)")]
    MissingPublisher,

    #[error(
        "publisher field `{0}` is invalid for modern project IDs: must be 3-50 characters, use only ASCII letters and numbers, may include single spaces or hyphens between words, and must start and end with a letter or number"
    )]
    NonCanonicalizablePublisher(Box<str>),

    #[error(
        "name field `{0}` is invalid for modern project IDs: must be 3-50 characters, use only letters and numbers, may include single spaces, hyphens, or dots between words, and must start and end with a letter or number"
    )]
    NonCanonicalizableName(Box<str>),

    #[error(
        "version field `{version}` is invalid for publishing: must be a valid Semantic Versioning 2.0 version ({source})"
    )]
    InvalidVersion {
        version: Box<str>,
        source: semver::Error,
    },

    #[error("missing license in project info (required for publishing)")]
    MissingLicense,

    #[error(
        "license field `{license}` is invalid for publishing: must be a valid SPDX license expression ({source})"
    )]
    InvalidLicense {
        license: Box<str>,
        source: spdx::error::ParseError,
    },

    #[error("invalid index URL `{url}` for publish endpoint: {reason}")]
    InvalidIndexUrl { url: Box<str>, reason: String },

    #[error(
        "no bearer token credentials configured for publish URL `{0}`; set SYSAND_CRED_<X> and SYSAND_CRED_<X>_BEARER_TOKEN with a matching URL pattern"
    )]
    MissingCredentials(Box<str>),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest_middleware::Error),

    #[error("server error ({0}): {1}")]
    ServerError(u16, String),

    #[error("authentication failed: {0}")]
    AuthError(String),

    #[error("conflict: package version already exists: {0}")]
    Conflict(String),

    #[error("bad request: {0}")]
    BadRequest(String),
}

#[derive(Debug)]
pub struct PublishResponse {
    pub status: u16,
    pub message: String,
    pub is_new_project: bool,
}

fn build_upload_url(index_url: &Url) -> Result<Url, PublishError> {
    if !matches!(index_url.scheme(), "http" | "https") {
        return Err(PublishError::InvalidIndexUrl {
            url: index_url.as_str().into(),
            reason: "URL scheme must be http or https".to_string(),
        });
    }

    if index_url.query().is_some() {
        return Err(PublishError::InvalidIndexUrl {
            url: index_url.as_str().into(),
            reason: "URL must not include a query component".to_string(),
        });
    }

    if index_url.fragment().is_some() {
        return Err(PublishError::InvalidIndexUrl {
            url: index_url.as_str().into(),
            reason: "URL must not include a fragment component".to_string(),
        });
    }

    let mut upload_url = index_url.clone();
    {
        let mut segments = upload_url.path_segments_mut().unwrap();
        segments.pop_if_empty();
        segments.extend(["api", "v1", "upload"]);
    }

    Ok(upload_url)
}

pub fn do_publish_kpar<P: AsRef<Utf8Path>>(
    kpar_path: P,
    index_url: Url,
    auth_policy: Arc<PublishHTTPAuthentication>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<PublishResponse, PublishError> {
    let kpar_path = kpar_path.as_ref();
    let header = crate::style::get_style_config().header;

    // Open and validate kpar
    let kpar_project = LocalKParProject::new_guess_root(kpar_path)
        .map_err(|e| PublishError::KparOpen(kpar_path.as_str().into(), e.to_string()))?;

    let (info, meta) = kpar_project
        .get_project()
        .map_err(|e| PublishError::KparOpen(kpar_path.as_str().into(), e.to_string()))?;

    let info = info.ok_or(PublishError::MissingInfo)?;
    let _meta = meta.ok_or(PublishError::MissingMeta)?;

    let publisher = info
        .publisher
        .as_deref()
        .ok_or(PublishError::MissingPublisher)?;
    let name = &info.name;
    let version = &info.version;
    let license = info
        .license
        .as_deref()
        .ok_or(PublishError::MissingLicense)?;
    if !is_canonicalizable_publisher_field_value(publisher) {
        return Err(PublishError::NonCanonicalizablePublisher(publisher.into()));
    }
    if !is_canonicalizable_name_field_value(name) {
        return Err(PublishError::NonCanonicalizableName(name.as_str().into()));
    }
    semver::Version::parse(version).map_err(|source| PublishError::InvalidVersion {
        version: version.as_str().into(),
        source,
    })?;
    spdx::Expression::parse(license).map_err(|source| PublishError::InvalidLicense {
        license: license.into(),
        source,
    })?;
    let normalized_publisher = canonicalize_modern_project_id_component(publisher);
    let normalized_name = canonicalize_modern_project_id_component(name);
    let purl = format!("pkg:sysand/{normalized_publisher}/{normalized_name}@{version}");

    let publishing = "Publishing";
    log::info!(
        "{header}{publishing:>12}{header:#} `{name}` {version} to {}",
        index_url
    );

    let file_name = kpar_path
        .file_name()
        .unwrap_or(kpar_path.as_str())
        .to_string();

    // Read kpar file bytes
    let file_bytes = std::fs::read(kpar_path)
        .map_err(|e| PublishError::KparRead(kpar_path.as_str().into(), e))?;

    let upload_url = build_upload_url(&index_url)?;

    match auth_policy.restricted.lookup(upload_url.as_str()) {
        GlobMapResult::NotFound => {
            return Err(PublishError::MissingCredentials(upload_url.as_str().into()));
        }
        GlobMapResult::Found(_, _) | GlobMapResult::Ambiguous(_) => {}
    }

    // Keep upload payload in `Bytes` so request retries clone cheaply.
    let file_bytes = Bytes::from(file_bytes);
    let upload_url_for_request = upload_url.clone();

    let request_builder = move |c: &reqwest_middleware::ClientWithMiddleware| {
        let file_part = reqwest::multipart::Part::stream(file_bytes.clone())
            .file_name(file_name.clone())
            .mime_str("application/octet-stream")
            .unwrap();

        let form = reqwest::multipart::Form::new()
            .text("purl", purl.clone())
            .part("file", file_part);

        c.post(upload_url_for_request.clone()).multipart(form)
    };

    let response = runtime.block_on(async {
        auth_policy
            .with_authentication(&client, &request_builder)
            .await
    })?;

    let status = response.status().as_u16();
    let response_url = response.url().to_string();
    let body = runtime.block_on(response.text()).unwrap_or_default();
    log::debug!(
        "publish response: request URL `{}`, final URL `{}`, status {}",
        upload_url,
        response_url,
        status
    );

    match status {
        200 => Ok(PublishResponse {
            status,
            message: body,
            is_new_project: false,
        }),
        201 => Ok(PublishResponse {
            status,
            message: body,
            is_new_project: true,
        }),
        401 | 403 => Err(PublishError::AuthError(body)),
        409 => Err(PublishError::Conflict(body)),
        400 | 404 => Err(PublishError::BadRequest(body)),
        _ => {
            log::warn!(
                "publish failed: request URL `{}`, final URL `{}`, status {}",
                upload_url,
                response_url,
                status
            );
            Err(PublishError::ServerError(status, body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PublishError, build_upload_url, canonicalize_modern_project_id_component,
        is_canonicalizable_name_field_value, is_canonicalizable_publisher_field_value,
    };
    use url::Url;

    #[test]
    fn publisher_field_canonicalizability() {
        assert!(is_canonicalizable_publisher_field_value("Acme Labs"));
        assert!(is_canonicalizable_publisher_field_value("ACME-LABS-42"));
        assert!(!is_canonicalizable_publisher_field_value("ab"));
        assert!(!is_canonicalizable_publisher_field_value("Acme  Labs"));
        assert!(!is_canonicalizable_publisher_field_value("Acme__Labs"));
        assert!(!is_canonicalizable_publisher_field_value("Acme."));
    }

    #[test]
    fn name_field_canonicalizability() {
        assert!(is_canonicalizable_name_field_value("My.Project Alpha"));
        assert!(is_canonicalizable_name_field_value("Alpha-2"));
        assert!(!is_canonicalizable_name_field_value("ab"));
        assert!(!is_canonicalizable_name_field_value("My..Project"));
        assert!(!is_canonicalizable_name_field_value("My__Project"));
        assert!(!is_canonicalizable_name_field_value(".Project"));
    }

    #[test]
    fn canonicalize_modern_project_id_component_preserves_dot() {
        assert_eq!(
            canonicalize_modern_project_id_component("My.Project Alpha"),
            "my.project-alpha"
        );
        assert_eq!(
            canonicalize_modern_project_id_component("ACME LABS"),
            "acme-labs"
        );
    }

    #[test]
    fn build_upload_url_appends_endpoint_path() {
        assert_eq!(
            build_upload_url(&Url::parse("https://example.org").unwrap())
                .unwrap()
                .as_str(),
            "https://example.org/api/v1/upload"
        );
        assert_eq!(
            build_upload_url(&Url::parse("https://example.org/").unwrap())
                .unwrap()
                .as_str(),
            "https://example.org/api/v1/upload"
        );
        assert_eq!(
            build_upload_url(&Url::parse("https://example.org/index").unwrap())
                .unwrap()
                .as_str(),
            "https://example.org/index/api/v1/upload"
        );
        assert_eq!(
            build_upload_url(&Url::parse("https://example.org/index/").unwrap())
                .unwrap()
                .as_str(),
            "https://example.org/index/api/v1/upload"
        );
    }

    #[test]
    fn build_upload_url_strips_query_and_fragment() {
        let err = build_upload_url(&Url::parse("https://example.org/index?x=1#frag").unwrap())
            .unwrap_err();
        assert!(matches!(err, PublishError::InvalidIndexUrl { .. }));
    }

    #[test]
    fn build_upload_url_rejects_non_http_scheme() {
        let err = build_upload_url(&Url::parse("ftp://example.org").unwrap()).unwrap_err();
        assert!(matches!(err, PublishError::InvalidIndexUrl { .. }));
    }

    #[test]
    fn build_upload_url_rejects_non_hierarchical_url() {
        let err = build_upload_url(&Url::parse("mailto:test@example.org").unwrap()).unwrap_err();
        assert!(matches!(err, PublishError::InvalidIndexUrl { .. }));
    }
}
