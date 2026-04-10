// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use bytes::Bytes;
use camino::Utf8Path;
use sha2::Digest;
use thiserror::Error;
use url::Url;

use crate::{
    auth::{ForceBearerAuth, HTTPAuthentication},
    project::{ProjectRead, local_kpar::LocalKParProject},
};

/// Defensive upper bound on kpar file size (100 MiB) to catch unexpected uploads by mistake.
const MAX_KPAR_PUBLISH_SIZE: u64 = 100 * 1024 * 1024;
/// Maximum number of characters to include when summarizing an error response body.
const MAX_ERROR_BODY_CHARS: usize = 1024;
/// Path segments appended to the index URL to form the upload endpoint.
const UPLOAD_ENDPOINT_SEGMENTS: [&str; 3] = ["api", "v1", "upload"];

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
        name,
        version,
        file_name,
        file_bytes,
        metadata,
    } = prepare_publish_payload(path)?;
    log::info!(
        "{header}{:>12}{header:#} `{name}` {version} to {index}",
        "Publishing",
    );

    // Stash the URL as a string for post-request logging; the `Url` itself
    // is moved into the closure since `with_authentication` may call it
    // multiple times and each `post` consumes the URL.
    let upload_url_for_log = upload_url.to_string();

    let build_request = move |c: &reqwest_middleware::ClientWithMiddleware| {
        let file_part = reqwest::multipart::Part::stream(file_bytes.clone())
            .file_name(file_name.clone())
            .mime_str("application/zip")
            .expect("hard-coded content type must be a valid MIME");
        let metadata_part = reqwest::multipart::Part::text(metadata.clone())
            .mime_str("application/json")
            .expect("hard-coded content type must be a valid MIME");

        let form = reqwest::multipart::Form::new()
            .part("metadata", metadata_part)
            .part("file", file_part);

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

    let mut upload_url = index.clone();
    {
        let mut segments = upload_url
            .path_segments_mut()
            .expect("http(s) URLs are hierarchical and must support mutable path segments");
        // Normalize both `https://host` and `https://host/`.
        segments.pop_if_empty();
    }

    // After normalization, reject URLs that already end with the upload path.
    let path_segments: Vec<_> = upload_url
        .path_segments()
        .expect("http(s) URLs are hierarchical and must support path segments")
        .collect();
    if path_segments.ends_with(&UPLOAD_ENDPOINT_SEGMENTS) {
        return Err(PublishError::InvalidIndexUrl {
            url: index.as_str().into(),
            reason: "URL must point to the index root; do not include `/api/v1/upload`".to_string(),
        });
    }

    {
        let mut segments = upload_url
            .path_segments_mut()
            .expect("http(s) URLs are hierarchical and must support mutable path segments");
        for segment in UPLOAD_ENDPOINT_SEGMENTS {
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

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest_middleware::Error),

    #[error("failed to read server response body: {0}")]
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
        "kpar file is unexpectedly large ({size} bytes, limit is {limit} bytes); verify you are publishing the correct file"
    )]
    KparTooLarge { size: u64, limit: u64 },
}

// --- Private helpers ---

struct PublishPreparation {
    name: String,
    version: String,
    file_name: String,
    // Keep upload payload in `Bytes` so request retries clone cheaply.
    file_bytes: Bytes,
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
    let purl = format!("pkg:sysand/{normalized_publisher}/{normalized_name}@{version}");

    let file_name = path.file_name().unwrap_or(path.as_str()).to_string();

    let file_size = std::fs::metadata(path)
        .map_err(|e| PublishError::KparRead(path.as_str().into(), e))?
        .len();
    if file_size > MAX_KPAR_PUBLISH_SIZE {
        return Err(PublishError::KparTooLarge {
            size: file_size,
            limit: MAX_KPAR_PUBLISH_SIZE,
        });
    }

    let file_bytes =
        std::fs::read(path).map_err(|e| PublishError::KparRead(path.as_str().into(), e))?;
    let sha256_digest = format!("{:x}", sha2::Sha256::digest(&file_bytes));
    let metadata = serde_json::json!({
        "purl": purl,
        "sha256_digest": sha256_digest,
    })
    .to_string();

    Ok(PublishPreparation {
        name: name.clone(),
        version: version.clone(),
        file_name,
        file_bytes: Bytes::from(file_bytes),
        metadata,
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
        401 | 403 => Err(PublishError::AuthError(summarize_error_body(body_bytes))),
        409 => Err(PublishError::Conflict(summarize_error_body(body_bytes))),
        400 => Err(PublishError::BadRequest(summarize_error_body(body_bytes))),
        404 => Err(PublishError::NotFound(summarize_error_body(body_bytes))),
        _ => {
            log::warn!(
                "publish failed: request URL `{}`, final URL `{}`, status {}",
                upload_url_for_log,
                response_url,
                status
            );
            Err(PublishError::ServerError {
                status,
                body: summarize_error_body(body_bytes),
            })
        }
    }
}

/// Validates a publisher or name field for modern project IDs.
///
/// Rules: 3-50 ASCII alphanumeric characters, with single separators (space,
/// hyphen, and optionally dot when `allow_dot` is true) allowed between words.
/// Must start and end with an alphanumeric character.
///
/// Publish-only; if additional surfaces need this, extract to a shared module.
fn is_valid_field(s: &str, allow_dot: bool) -> bool {
    if !s.is_ascii() {
        return false;
    }

    let bytes = s.as_bytes();
    if !(3..=50).contains(&bytes.len()) {
        return false;
    }

    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return false;
    }

    for i in 1..(bytes.len() - 1) {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() {
            continue;
        }

        let is_separator = b == b'-' || b == b' ' || (allow_dot && b == b'.');
        if !is_separator {
            return false;
        }

        if !bytes[i - 1].is_ascii_alphanumeric() || !bytes[i + 1].is_ascii_alphanumeric() {
            return false;
        }
    }

    true
}

fn is_valid_publisher(s: &str) -> bool {
    is_valid_field(s, false)
}

fn is_valid_name(s: &str) -> bool {
    is_valid_field(s, true)
}

fn normalize_field(s: &str) -> String {
    s.to_ascii_lowercase().replace(' ', "-")
}

fn summarize_error_body(body_bytes: &[u8]) -> String {
    if body_bytes.is_empty() {
        return "empty response body".to_string();
    }

    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body_bytes) {
        let error = json.get("error").and_then(|v| v.as_str());
        let detail = json.get("detail").and_then(|v| v.as_str());
        let message = match (error, detail) {
            (Some(error), Some(detail)) => format!("{error}: {detail}"),
            (Some(error), None) => error.to_string(),
            (None, Some(detail)) => detail.to_string(),
            (None, None) => String::new(),
        };
        if !message.is_empty() {
            return summarize_error_text(&message);
        }
    }

    match std::str::from_utf8(body_bytes) {
        Ok(text) => {
            if text.chars().any(|c| c.is_control() && !c.is_whitespace()) {
                return format!(
                    "unexpected non-text error response ({} bytes)",
                    body_bytes.len()
                );
            }
            summarize_error_text(text)
        }
        Err(_) => format!(
            "unexpected non-text error response ({} bytes)",
            body_bytes.len()
        ),
    }
}

fn summarize_error_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "empty response body".to_string();
    }

    let mut summarized = trimmed.to_string();
    if summarized.len() > MAX_ERROR_BODY_CHARS {
        let mut cutoff = MAX_ERROR_BODY_CHARS;
        while !summarized.is_char_boundary(cutoff) {
            cutoff -= 1;
        }
        summarized.truncate(cutoff);
    }
    if summarized.len() < trimmed.len() {
        summarized.push_str(" ... [truncated]");
    }

    summarized
}

#[cfg(test)]
#[path = "./publish_tests.rs"]
mod tests;
