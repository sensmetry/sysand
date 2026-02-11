// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use camino::Utf8Path;
use thiserror::Error;

use crate::{
    auth::HTTPAuthentication,
    project::{ProjectRead, local_kpar::LocalKParProject},
};

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

pub fn do_publish_kpar<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    kpar_path: P,
    index_url: &str,
    auth_policy: Arc<Policy>,
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

    let name = &info.name;
    let version = &info.version;
    let purl = format!("pkg:sysand/{name}@{version}");

    let publishing = "Publishing";
    log::info!("{header}{publishing:>12}{header:#} `{name}` {version} to {index_url}");

    // Read kpar file bytes
    let file_bytes = std::fs::read(kpar_path)
        .map_err(|e| PublishError::KparRead(kpar_path.as_str().into(), e))?;

    let file_name = kpar_path.file_name().unwrap_or("package.kpar").to_string();

    let upload_url = format!("{}/api/v1/upload", index_url.trim_end_matches('/'));

    // Wrap in Arc for the 'static bound on the with_authentication closure
    let file_bytes = Arc::new(file_bytes);
    let file_name = Arc::new(file_name);
    let upload_url = Arc::new(upload_url);
    let purl = Arc::new(purl);

    let request_builder = move |c: &reqwest_middleware::ClientWithMiddleware| {
        let file_part = reqwest::multipart::Part::bytes((*file_bytes).clone())
            .file_name((*file_name).clone())
            .mime_str("application/octet-stream")
            .expect("valid mime type");

        let form = reqwest::multipart::Form::new()
            .text("purl", (*purl).clone())
            .part("file", file_part);

        c.post(upload_url.as_str()).multipart(form)
    };

    let response = runtime.block_on(async {
        auth_policy
            .with_authentication(&client, &request_builder)
            .await
    })?;

    let status = response.status().as_u16();
    let body = runtime.block_on(response.text()).unwrap_or_default();

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
        _ => Err(PublishError::ServerError(status, body)),
    }
}
