// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::sync::Arc;

use bytes::Bytes;
use camino::Utf8Path;
use serde::Deserialize;
use sha2::Digest;
use thiserror::Error;
use url::Url;

use crate::{
    auth::{ForceBearerAuth, HTTPAuthentication},
    project::{ProjectRead, local_kpar::LocalKParProject},
};

/// Defensive upper bound on kpar file size (100 MiB) to catch unexpected uploads by mistake.
const MAX_KPAR_PUBLISH_SIZE: u64 = 100 * 1024 * 1024;
/// Path appended to the index URL to form the upload endpoint.
const UPLOAD_ENDPOINT_PATH: &str = "/api/v1/upload";

pub fn do_publish<P: AsRef<Utf8Path>>(
    path: P,
    index: Url,
    auth: ForceBearerAuth,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<PublishResponse, PublishError> {
    let path = path.as_ref();
    let header = crate::style::get_style_config().header;
    let upload_url = build_upload_url(&index)?;
    let PublishPreparation {
        purl_versioned,
        metadata,
        kpar_bytes,
    } = prepare_publish_payload(path)?;
    log::info!(
        "{header}{:>12}{header:#} `{purl_versioned}` to {index}",
        "Publishing",
    );

    // Stash the URL as a string for post-request logging; the `Url` itself
    // is moved into the closure since `with_authentication` may call it
    // multiple times and each `post` consumes the URL.
    let upload_url_for_log = upload_url.to_string();

    let build_request = move |c: &reqwest_middleware::ClientWithMiddleware| {
        let metadata_part = reqwest::multipart::Part::text(metadata.clone())
            .mime_str("application/json")
            .unwrap();
        let kpar_part = reqwest::multipart::Part::stream(kpar_bytes.clone())
            // we declare an arbitrary filename to help server side libraries
            // make reasonable assumptions reading the POST request, such as not
            // trying to parse the binary data as UTF-8 or similar
            .file_name("project.kpar")
            .mime_str("application/zip")
            .unwrap();

        let form = reqwest::multipart::Form::new()
            .part("metadata", metadata_part)
            .part("kpar", kpar_part);

        c.post(upload_url.clone()).multipart(form)
    };

    let response =
        runtime.block_on(async { auth.with_authentication(&client, &build_request).await })?;

    let status = response.status().as_u16();
    let response_url = response.url().to_string();
    let body_bytes = runtime
        .block_on(response.bytes())
        .map_err(PublishError::ResponseBody)?;
    log::debug!(
        "publish response: request URL `{}`, final URL `{}`, status {}",
        upload_url_for_log,
        response_url,
        status
    );

    map_publish_response(status, &body_bytes, &upload_url_for_log, &response_url)
}

pub fn build_upload_url(index: &Url) -> Result<Url, PublishError> {
    if !matches!(index.scheme(), "http" | "https") {
        return Err(PublishError::InvalidIndexUrl {
            url: index.as_str().into(),
            reason: "URL scheme must be http or https".to_string(),
        });
    }

    if index.query().is_some() {
        return Err(PublishError::InvalidIndexUrl {
            url: index.as_str().into(),
            reason: "URL must not include a query component".to_string(),
        });
    }

    if index.fragment().is_some() {
        return Err(PublishError::InvalidIndexUrl {
            url: index.as_str().into(),
            reason: "URL must not include a fragment component".to_string(),
        });
    }

    let mut upload_url = index.to_owned();
    {
        // Guaranteed for validated http(s) URLs.
        let mut segments = upload_url.path_segments_mut().unwrap();
        // Normalize both `https://host` and `https://host/`.
        segments.pop_if_empty();
    }

    // After normalization, reject URLs that already end with the upload path.
    if upload_url.path().ends_with(UPLOAD_ENDPOINT_PATH) {
        return Err(PublishError::InvalidIndexUrl {
            url: index.as_str().into(),
            reason: "URL must point to the index root; do not include `/api/v1/upload`".to_string(),
        });
    }

    {
        let mut segments = upload_url.path_segments_mut().unwrap();
        for segment in UPLOAD_ENDPOINT_PATH.trim_start_matches('/').split('/') {
            segments.push(segment);
        }
    }

    Ok(upload_url)
}

#[derive(Debug)]
pub struct PublishResponse {
    pub status: u16,
    pub message: String,
    pub is_new_project: bool,
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
    InvalidPublisher(Box<str>),

    #[error(
        "name field `{0}` is invalid for modern project IDs: must be 3-50 characters, use only ASCII letters and numbers, may include single spaces, hyphens, or dots between words, and must start and end with a letter or number"
    )]
    InvalidName(Box<str>),

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

    #[error("HTTP request failed: {0:#?}")]
    Http(#[from] reqwest_middleware::Error),

    #[error("failed to read server response body: {0:#?}")]
    ResponseBody(#[source] reqwest::Error),

    #[error("server error ({status}): {body}")]
    ServerError { status: u16, body: String },

    #[error("authentication failed: {0}")]
    AuthError(String),

    #[error("conflict: package version already exists: {0}")]
    Conflict(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("publish endpoint not found: {0}")]
    NotFound(String),

    #[error(
        "KPAR file is unexpectedly large ({size} bytes, limit is {limit} bytes); verify you are publishing the correct file"
    )]
    KparTooLarge { size: u64, limit: u64 },
}

// --- Private helpers ---

struct PublishPreparation {
    purl_versioned: String,
    // Keep upload payload in `Bytes` so request retries clone cheaply.
    kpar_bytes: Bytes,
    metadata: String,
}

/// Reads and validates a `.kpar` file, returning the upload payload and metadata.
fn prepare_publish_payload(path: &Utf8Path) -> Result<PublishPreparation, PublishError> {
    // Open and validate kpar.
    let kpar_project = LocalKParProject::new_guess_root(path)
        .map_err(|e| PublishError::KparOpen(path.as_str().into(), e.to_string()))?;

    let (info, meta) = kpar_project
        .get_project()
        .map_err(|e| PublishError::KparOpen(path.as_str().into(), e.to_string()))?;

    let info = info.ok_or(PublishError::MissingInfo)?;
    // Validate that metadata exists; contents are not used during upload.
    _ = meta.ok_or(PublishError::MissingMeta)?;

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
    if !is_valid_publisher(publisher) {
        return Err(PublishError::InvalidPublisher(publisher.into()));
    }
    if !is_valid_name(name) {
        return Err(PublishError::InvalidName(name.as_str().into()));
    }
    semver::Version::parse(version).map_err(|source| PublishError::InvalidVersion {
        version: version.as_str().into(),
        source,
    })?;
    spdx::Expression::parse(license).map_err(|source| PublishError::InvalidLicense {
        license: license.into(),
        source,
    })?;
    let normalized_publisher = normalize_field(publisher);
    let normalized_name = normalize_field(name);
    let purl_versioned = format!("pkg:sysand/{normalized_publisher}/{normalized_name}@{version}");

    let file_size = std::fs::metadata(path)
        .map_err(|e| PublishError::KparRead(path.as_str().into(), e))?
        .len();
    if file_size > MAX_KPAR_PUBLISH_SIZE {
        return Err(PublishError::KparTooLarge {
            size: file_size,
            limit: MAX_KPAR_PUBLISH_SIZE,
        });
    }

    let kpar_bytes =
        std::fs::read(path).map_err(|e| PublishError::KparRead(path.as_str().into(), e))?;
    let sha256_digest = format!("{:x}", sha2::Sha256::digest(&kpar_bytes));
    let metadata = serde_json::json!({
        "normalized_publisher": normalized_publisher,
        "normalized_name": normalized_name,
        "version": version,
        "license": license,
        "kpar_sha256_digest": sha256_digest,
    })
    .to_string();

    Ok(PublishPreparation {
        purl_versioned,
        metadata,
        kpar_bytes: Bytes::from(kpar_bytes),
    })
}

/// Maps an HTTP status and body to a `PublishResponse` or `PublishError`.
fn map_publish_response(
    status: u16,
    body_bytes: &[u8],
    upload_url_for_log: &str,
    response_url: &str,
) -> Result<PublishResponse, PublishError> {
    match status {
        200 => Ok(PublishResponse {
            status,
            message: String::from_utf8_lossy(body_bytes).into_owned(),
            is_new_project: false,
        }),
        201 => Ok(PublishResponse {
            status,
            message: String::from_utf8_lossy(body_bytes).into_owned(),
            is_new_project: true,
        }),
        400 => Err(PublishError::BadRequest(error_body_to_string(body_bytes))),
        401 | 403 => Err(PublishError::AuthError(error_body_to_string(body_bytes))),
        404 => Err(PublishError::NotFound(error_body_to_string(body_bytes))),
        409 => Err(PublishError::Conflict(error_body_to_string(body_bytes))),
        _ => {
            log::warn!(
                "publish failed: request URL `{}`, final URL `{}`, status {}",
                upload_url_for_log,
                response_url,
                status
            );
            Err(PublishError::ServerError {
                status,
                body: error_body_to_string(body_bytes),
            })
        }
    }
}

use crate::purl::{is_valid_name, is_valid_publisher, normalize_field};

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

fn error_body_to_string(body_bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(body_bytes);
    let trimmed = text.trim();

    if trimmed.is_empty() {
        return "no error details provided".to_string();
    }

    serde_json::from_str::<ErrorResponse>(trimmed)
        .map(|error| error.error)
        .unwrap_or_else(|_| trimmed.to_string())
}

#[cfg(test)]
#[path = "./publish_tests.rs"]
mod tests;
