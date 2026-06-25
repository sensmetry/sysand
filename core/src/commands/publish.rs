// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    collections::{HashMap, hash_map::Entry},
    env,
    io::{self, Read as _},
    sync::Arc,
};

use bytes::Bytes;
use camino::Utf8Path;
use reqwest::header;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use url::Url;
use zip::result::ZipError;

use crate::{
    auth::{ForceBearerAuth, GlobMap, GlobMapResult, HTTPAuthentication},
    env::discovery::{HttpBaseUrlShapeError, validate_http_base_url_shape},
    include::{IncludeError, extract_symbols},
    model::{
        InterchangeProjectUsageRaw, InterchangeProjectValidationError, KERML_METAMODEL_PREFIX,
        KerMlChecksumAlg, SYSML_METAMODEL_PREFIX,
    },
    project::{
        ProjectRead,
        local_kpar::{LocalKParError, LocalKParProjectRaw},
        utils::{FsIoError, wrapfs},
    },
    purl::{
        SysandPurlError, is_valid_unnormalized_name, is_valid_unnormalized_publisher,
        normalize_field, parse_sysand_purl,
    },
    symbols::Language,
    utils::{
        RelativePathKind, RelativeUnixPathError, license_file_stems, parse_relative_unix_path,
        sha256_lowercase_hex,
    },
};

/// Defensive upper bound on kpar file size (100 MiB) to catch unexpected uploads by mistake.
const MAX_KPAR_PUBLISH_SIZE: u64 = 100 * 1024 * 1024;
/// Path appended to the API root to form the upload endpoint. The
/// trailing/leading slashes are omitted here so the value composes
/// cleanly with `Url::join`, which treats a base ending in `/` as a
/// directory.
const UPLOAD_ENDPOINT_PATH: &str = "v1/upload";
const TRUSTED_PUBLISHING_EXCHANGE_PATH: &str = "v1/oidc/token";
const TRUSTED_PUBLISHING_AUDIENCE: &str = "sysand";

/// How publish should use CI trusted publishing to acquire a bearer
/// token when no explicit publish bearer credential matches.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TrustedPublishingMode {
    /// Detect a supported CI provider from the supplied environment.
    #[default]
    Auto,
    /// Do not use trusted publishing.
    Never,
    /// Require GitHub Actions trusted publishing.
    Github,
    /// Require GitLab CI trusted publishing.
    Gitlab,
}

/// Values from the CI environment that trusted publishing can use.
///
/// Non-CLI callers can construct this directly instead of relying on process
/// environment variables. The CLI uses [`TrustedPublishingEnvironment::from_env`]
/// as a convenience adapter.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TrustedPublishingEnvironment {
    pub github_request_token: Option<String>,
    pub github_request_url: Option<String>,
    pub gitlab_oidc_token: Option<String>,
}

impl TrustedPublishingEnvironment {
    /// Capture the trusted-publishing environment variables used by GitHub
    /// Actions and GitLab CI.
    pub fn from_env() -> Self {
        Self {
            github_request_token: env_var_nonempty("ACTIONS_ID_TOKEN_REQUEST_TOKEN"),
            github_request_url: env_var_nonempty("ACTIONS_ID_TOKEN_REQUEST_URL"),
            gitlab_oidc_token: env_var_nonempty("GITLAB_OIDC_TOKEN"),
        }
    }
}

/// Trusted publishing providers whose CI environments publish can recognize
/// and exchange for a Sysand index bearer token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedPublishingProvider {
    Github,
    Gitlab,
}

impl TrustedPublishingProvider {
    fn name(self) -> &'static str {
        match self {
            TrustedPublishingProvider::Github => "github",
            TrustedPublishingProvider::Gitlab => "gitlab",
        }
    }
}

pub fn do_publish(
    prepared: PublishPreparation,
    discovery_root: Url,
    api_root: Url,
    auth: ForceBearerAuth,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<PublishResponse, PublishError> {
    let header = crate::style::get_style_config().header;
    // Caller is expected to have run discovery and passed the resolved
    // `api_root`. `discovery_root` is the user-facing URL (what was
    // passed as `--index`) and is kept only so log messages match what
    // the user configured — the actual upload targets `api_root`.
    let upload_url = build_upload_url(&api_root)?;
    let PublishPreparation {
        norm_publisher: publisher,
        norm_name: name,
        version,
        metadata,
        kpar_bytes,
    } = prepared;
    log::info!(
        "{header}{:>12}{header:#} {publisher}/{name} v{version} to {discovery_root}",
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

/// Which root is being validated — selects the error variant so the
/// message names the spec concept the URL came from.
#[derive(Debug, Clone, Copy)]
pub enum EndpointKind {
    /// User-supplied URL (`--index`).
    DiscoveryRoot,
    /// Resolved URL coming back from the discovery document.
    ApiRoot,
}

/// Validate the shape of an index-server endpoint URL before the network
/// step. Applies to both the user-supplied discovery root (pre-discovery)
/// and the resolved `api_root` that comes back from discovery.
pub fn validate_endpoint_url_shape(url: &Url, kind: EndpointKind) -> Result<(), PublishError> {
    let err = |reason: String| -> PublishError {
        match kind {
            EndpointKind::DiscoveryRoot => PublishError::InvalidDiscoveryRoot {
                url: url.as_str().into(),
                reason,
            },
            EndpointKind::ApiRoot => PublishError::InvalidApiRoot {
                url: url.as_str().into(),
                reason,
            },
        }
    };
    validate_http_base_url_shape(url).map_err(|e| {
        err(match e {
            HttpBaseUrlShapeError::UnsupportedScheme => "URL scheme must be http or https",
            HttpBaseUrlShapeError::Userinfo => "URL must not include username or password",
        }
        .to_string())
    })?;
    if url.query().is_some() {
        return Err(err("URL must not include a query component".to_string()));
    }
    if url.fragment().is_some() {
        return Err(err("URL must not include a fragment component".to_string()));
    }
    // Reject a URL that already names the upload endpoint. Catches the
    // common mistake of pasting the full upload URL into `--index`,
    // which would otherwise either compose to `v1/upload/v1/upload`
    // (after discovery defaulted `api_root` to the discovery root) or
    // send a discovery request to a path that can never serve one.
    if url.path().trim_end_matches('/').ends_with("v1/upload") {
        return Err(err(
            "URL must be a discovery root or `api_root`, not the `v1/upload` endpoint".to_string(),
        ));
    }
    Ok(())
}

/// Build the `POST` URL for the publish endpoint from a resolved
/// `api_root`. The caller is responsible for having resolved the API
/// root via `sysand-index-config.json`; this function appends the
/// publish endpoint path to `api_root` as given and does not prepend
/// any `/api/` segment — that belongs to the API root itself.
pub fn build_upload_url(api_root: &Url) -> Result<Url, PublishError> {
    // The `v1/upload` suffix rejection is part of shape validation.
    validate_endpoint_url_shape(api_root, EndpointKind::ApiRoot)?;

    Ok(crate::env::discovery::with_trailing_slash(api_root.clone())
        .join(UPLOAD_ENDPOINT_PATH)
        .unwrap())
}

/// Resolve the bearer token used for publishing.
///
/// Explicit publish bearer credentials take priority. If no explicit bearer
/// matches, trusted publishing may acquire a short-lived index token according
/// to `mode` and `env`. `api_root` is the resolved API root from index
/// discovery; the upload URL used for credential matching is derived from it.
pub fn resolve_publish_bearer(
    bearer_map: &GlobMap<ForceBearerAuth>,
    api_root: &Url,
    mode: TrustedPublishingMode,
    env: &TrustedPublishingEnvironment,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
) -> Result<ForceBearerAuth, PublishError> {
    let upload_url = build_upload_url(api_root)?;
    match bearer_map.lookup(upload_url.as_str()) {
        GlobMapResult::Found(_, token) => Ok(token.clone()),
        GlobMapResult::Ambiguous(candidates) => Err(PublishError::AmbiguousPublishBearer {
            upload_url: upload_url.as_str().into(),
            candidates: candidates.len(),
        }),
        GlobMapResult::NotFound => {
            match acquire_trusted_publishing_bearer(mode, env, api_root, client, runtime)? {
                Some(token) => Ok(token),
                None => Err(PublishError::NoPublishBearer {
                    upload_url: upload_url.as_str().into(),
                }),
            }
        }
    }
}

/// Resolve a publish bearer token from trusted publishing, returning `None`
/// when the selected mode intentionally leaves publish credential selection to
/// explicit bearer credentials.
fn acquire_trusted_publishing_bearer(
    mode: TrustedPublishingMode,
    env: &TrustedPublishingEnvironment,
    api_root: &Url,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
) -> Result<Option<ForceBearerAuth>, PublishError> {
    let Some(provider) = select_trusted_publishing_provider(mode, env)? else {
        return Ok(None);
    };

    log::debug!("trusted publishing: using {provider:?}");
    let provider_token = match provider {
        TrustedPublishingProvider::Github => acquire_github_oidc_token(env, client, runtime)?,
        TrustedPublishingProvider::Gitlab => gitlab_oidc_token_from_env(env)?,
    };
    let index_token =
        exchange_oidc_token_for_index_token(api_root, &provider_token, client, runtime)?;

    Ok(Some(ForceBearerAuth::new(index_token)))
}

/// Convert trusted-publishing mode and environment into one concrete provider.
/// In `auto` mode incomplete environments are ignored, but two complete
/// provider environments are rejected to avoid guessing which identity to use.
fn select_trusted_publishing_provider(
    mode: TrustedPublishingMode,
    env: &TrustedPublishingEnvironment,
) -> Result<Option<TrustedPublishingProvider>, PublishError> {
    match mode {
        TrustedPublishingMode::Never => Ok(None),
        TrustedPublishingMode::Github => {
            ensure_github_env(env)?;
            Ok(Some(TrustedPublishingProvider::Github))
        }
        TrustedPublishingMode::Gitlab => {
            ensure_gitlab_env(env)?;
            Ok(Some(TrustedPublishingProvider::Gitlab))
        }
        TrustedPublishingMode::Auto => {
            let github = github_env_complete(env);
            let gitlab = gitlab_env_complete(env);
            match (github, gitlab) {
                (true, true) => Err(PublishError::MultipleTrustedPublishingProviders),
                (true, false) => Ok(Some(TrustedPublishingProvider::Github)),
                (false, true) => Ok(Some(TrustedPublishingProvider::Gitlab)),
                (false, false) => Ok(None),
            }
        }
    }
}

/// Whether the GitHub Actions environment exposes the complete OIDC request
/// contract needed to mint a provider token.
fn github_env_complete(env: &TrustedPublishingEnvironment) -> bool {
    option_string_nonempty(&env.github_request_token).is_some()
        && option_string_nonempty(&env.github_request_url).is_some()
}

/// Whether GitLab CI has injected the configured ID token into the expected
/// job variable.
fn gitlab_env_complete(env: &TrustedPublishingEnvironment) -> bool {
    option_string_nonempty(&env.gitlab_oidc_token).is_some()
}

/// Validate that forced GitHub trusted publishing has all runner-provided
/// variables needed to request the GitHub OIDC token.
fn ensure_github_env(env: &TrustedPublishingEnvironment) -> Result<(), PublishError> {
    let missing: Vec<&'static str> = [
        (
            "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
            option_string_nonempty(&env.github_request_token),
        ),
        (
            "ACTIONS_ID_TOKEN_REQUEST_URL",
            option_string_nonempty(&env.github_request_url),
        ),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.is_none().then_some(name))
    .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(PublishError::MissingTrustedPublishingEnvironment {
            provider: TrustedPublishingProvider::Github,
            variables: missing.into_boxed_slice(),
        })
    }
}

/// Validate that forced GitLab trusted publishing has the configured ID token
/// available in the job environment.
fn ensure_gitlab_env(env: &TrustedPublishingEnvironment) -> Result<(), PublishError> {
    if gitlab_env_complete(env) {
        Ok(())
    } else {
        Err(PublishError::MissingTrustedPublishingEnvironment {
            provider: TrustedPublishingProvider::Gitlab,
            variables: Box::new(["GITLAB_OIDC_TOKEN"]),
        })
    }
}

/// Read an environment variable while treating an empty string the same as an
/// unset variable.
fn env_var_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

/// Borrow an optional string only when it contains a non-empty value.
fn option_string_nonempty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

/// Read the GitLab CI OIDC token that GitLab injects when the job declares an
/// `id_tokens` entry for `GITLAB_OIDC_TOKEN`.
fn gitlab_oidc_token_from_env(env: &TrustedPublishingEnvironment) -> Result<String, PublishError> {
    option_string_nonempty(&env.gitlab_oidc_token)
        .map(ToOwned::to_owned)
        .ok_or_else(|| PublishError::MissingTrustedPublishingEnvironment {
            provider: TrustedPublishingProvider::Gitlab,
            variables: Box::new(["GITLAB_OIDC_TOKEN"]),
        })
}

/// Ask the GitHub Actions runner OIDC endpoint for a token with the `sysand`
/// audience and return the JSON `value` field from the response.
fn acquire_github_oidc_token(
    env: &TrustedPublishingEnvironment,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
) -> Result<String, PublishError> {
    ensure_github_env(env)?;
    let request_token = option_string_nonempty(&env.github_request_token).unwrap();
    let mut request_url = Url::parse(option_string_nonempty(&env.github_request_url).unwrap())
        .map_err(|source| PublishError::InvalidGithubOidcRequestUrl { source })?;
    request_url
        .query_pairs_mut()
        .append_pair("audience", TRUSTED_PUBLISHING_AUDIENCE);

    let response = runtime
        .block_on(async {
            client
                .get(request_url)
                .header(header::AUTHORIZATION, format!("bearer {request_token}"))
                .send()
                .await
        })
        .map_err(|source| PublishError::TrustedPublishingHttp {
            context: "GitHub OIDC token request",
            source,
        })?;

    let status = response.status();
    let body = runtime.block_on(response.bytes()).map_err(|source| {
        PublishError::TrustedPublishingResponseBody {
            context: "GitHub OIDC token response",
            source,
        }
    })?;
    if !status.is_success() {
        return Err(PublishError::TrustedPublishingProviderHttpStatus {
            provider: TrustedPublishingProvider::Github,
            status: status.as_u16(),
        });
    }

    json_string_field(&body, "value", "GitHub OIDC token response")
}

/// Exchange a provider-issued OIDC token at the resolved index API root and
/// return the short-lived Sysand bearer token from the response.
fn exchange_oidc_token_for_index_token(
    api_root: &Url,
    oidc_token: &str,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
) -> Result<String, PublishError> {
    let exchange_url = crate::env::discovery::with_trailing_slash(api_root.clone())
        .join(TRUSTED_PUBLISHING_EXCHANGE_PATH)
        .unwrap();
    let body = serde_json::json!({ "token": oidc_token }).to_string();

    let response = runtime
        .block_on(async {
            client
                .post(exchange_url.clone())
                .header(header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await
        })
        .map_err(|source| PublishError::TrustedPublishingHttp {
            context: "Sysand trusted publishing token exchange",
            source,
        })?;

    let status = response.status();
    let body = runtime.block_on(response.bytes()).map_err(|source| {
        PublishError::TrustedPublishingResponseBody {
            context: "Sysand trusted publishing token exchange response",
            source,
        }
    })?;
    if !status.is_success() {
        return Err(PublishError::TrustedPublishingExchangeHttpStatus {
            url: exchange_url.as_str().into(),
            status: status.as_u16(),
        });
    }

    json_string_field(
        &body,
        "token",
        "Sysand trusted publishing token exchange response",
    )
}

/// Extract a required non-empty string field from a small JSON response body.
fn json_string_field(
    bytes: &[u8],
    field: &'static str,
    context: &'static str,
) -> Result<String, PublishError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|source| PublishError::MalformedJsonResponse { context, source })?;
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(PublishError::MissingJsonField { context, field })
}

#[derive(Debug)]
pub struct PublishResponse {
    pub status: u16,
    pub message: String,
    pub is_new_project: bool,
}

// TODO: link to https://docs.sysand.com/index/reference/kpar-archive-validation upon encountering
// any validation error; maybe from CLI, maybe from here?
// TODO: add help in CLI that knowns which commands can be used to fix some issues
#[derive(Error, Debug)]
pub enum PublishError {
    #[error(
        "archive contains a file with executable permisions `{path}`;
        archive containing executable files cannot be published for security reasons"
    )]
    ExecInArchive { path: Box<str> },
    #[error("archive is corrupt: it contains overlapping files")]
    OverlappingFiles,
    #[error(
        "archive is corrupt: directory entry for `{path}`
        is marked as compressed with {comp}, but directory entries\n\
        do not have any content to compress"
    )]
    CompressedDirEntry {
        path: Box<str>,
        comp: zip::CompressionMethod,
    },
    #[error("archive contains an item with an invalid path")]
    InvalidPathInArchive(#[source] RelativeUnixPathError),
    #[error(
        "archive contains a file `{path}` which uses unsupported\n\
        {comp} compression; published archives currently must use DEFLATE\n\
        compression for all files"
    )]
    UnsupportedCompression {
        path: Box<str>,
        comp: zip::CompressionMethod,
    },
    #[error(
        "archive contains an encrypted file `{path}`;
        archives with encrypted files cannot be published"
    )]
    Encrypted { path: Box<str> },
    #[error(
        "archive contains a symbolic link `{path}`; symbolic links
        are forbidden for security reasons"
    )]
    Symlink { path: Box<str> },
    #[error(
        "metadata indicates that project file `{path}`\n\
        exports `{symbol}`, but it does not; it exports:\n\
        {actual_symbols:?}"
    )]
    NonexistentSymbolExported {
        path: Box<str>,
        symbol: Box<str>,
        actual_symbols: Box<[String]>,
    },
    #[error(
        "project metadata has incorrect checksum for source file `{path}`:\n\
        expected: {expected}\n  actual: {actual}"
    )]
    IncorrectFileChecksum {
        path: Box<str>,
        expected: Box<str>,
        actual: Box<str>,
    },
    #[error(
        "unsupported checksum algorithm `{alg}` for file `{path}`\n\
        only SHA256 is currently supported"
    )]
    UnsupportedFileChecksumType {
        path: Box<str>,
        alg: KerMlChecksumAlg,
    },
    #[error(
        "archive contains unexpected file `{path}`,\n\
        which is not mentioned in project metadata and is not an expected\n\
        license/readme/changelog file; remove this file from the archive"
    )]
    UnexpectedFile { path: Box<str> },
    #[error("project does not include `checksum` field in `.meta.json`")]
    MissingChecksum,
    #[error(
        "project doesn't list any source files (empty `checksum` field in\n\
        `.meta.json`); the project is not useful if it has no files\n\
        (did you forget to include them?)"
    )]
    EmptyChecksum,
    #[error("failed to index file")]
    IndexFail {
        source: IncludeError<LocalKParError>,
    },
    #[error(
        "project does not include source file `{path}`,\n\
        which is mentioned by `checksum` field of `.meta.json`"
    )]
    MissingFile { path: Box<str> },
    #[error("project metadata does not include a checksum for source file `{path}`")]
    MissingChecksumForFile { path: Box<str> },
    #[error("project includes source file `{path}`, which does not have expected\n\
        file extension `{}`; project metamodel `{metamodel_iri}`\n\
        requires all source files to be {}", metamodel.file_ext(), metamodel.lang())]
    IncorrectFileFormat {
        path: Box<str>,
        metamodel_iri: Box<str>,
        metamodel: AllowedMetamodelKind,
    },
    #[error(
        "archive does not include license file `{path}`, required because\n\
        the project's license is `{license}`, and every license/exception\n\
        must have its corresponding file; it is recommended to use\n\
        SPDX license text files from https://spdx.org/licenses/ or\n\
        https://github.com/spdx/license-list-data/tree/main/text\n\
        just note that some of them have placeholder copyright\n\
        holder/dates in the text that should be replaced"
    )]
    MissingLicenseFile { path: Box<str>, license: Box<str> },
    #[error(
        "metamodel `{metamodel}` cannot be used; only SysML/KerML\n\
        metamodels are currently allowed in the index"
    )]
    UnsupportedMetamodel { metamodel: String },
    #[error("project does not have a metamodel set")]
    MissingMetamodel,
    #[error("project metamodel `{metamodel}` has invalid version `{version}`")]
    InvalidMetamodelVersion {
        metamodel: Box<str>,
        version: Box<str>,
    },
    #[error("project usage includes unknown standard library `{name}`")]
    UnknownStdLib { name: Box<str> },
    #[error("project usage of standard library `{name}` specifies invalid version `{std_version}`")]
    InvalidStdLibVersion {
        name: Box<str>,
        std_version: Box<str>,
    },
    #[error(
        "standard library usage `{name}`\n\
        has a version constraint `{vc}`, but it is meaningless for direct URL usages"
    )]
    StdWithVersionConstraint { name: Box<str>, vc: Box<str> },
    #[error("invalid Sysand project identifier `{name}`")]
    InvalidPurl {
        name: Box<str>,
        source: SysandPurlError,
    },
    #[error(
        "project usage includes `{name}`,\n\
        which is neither in the index nor a SysML/KerML standard library"
    )]
    DisallowedUsage { name: Box<str> },
    #[error("KPAR's `.{name}.json` is invalid")]
    InfoMetaValidation {
        name: &'static str,
        source: InterchangeProjectValidationError,
    },
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("failed to read KPAR `{0}`")]
    KparRead(Box<str>, #[source] LocalKParError),
    #[error("failed to read KPAR `{0}`")]
    KparReadZip(Box<str>, #[source] ZipError),

    #[error("failed to read file `{path}` from the archive")]
    KparFileRead { path: Box<str>, source: ZipError },
    #[error("failed to read file `{path}` from the archive")]
    KparFileReadIo { path: Box<str>, source: io::Error },

    #[error(
        "archive `{kpar_path}` does not contain a project at its root,\n\
         the project is at `{root_in_kpar}` within the archive;\n\
         project must be at the root of the archive for publishing"
    )]
    ProjectNotAtRoot {
        kpar_path: Box<str>,
        root_in_kpar: Box<str>,
    },

    #[error("missing project info in kpar")]
    MissingInfo,

    #[error("missing project metadata in kpar")]
    MissingMeta,

    #[error(
        "missing publisher in project info; publisher has\n\
        to be set for publishing"
    )]
    MissingPublisher,

    #[error(
        "publisher `{0}` cannot be used for publishing;\n\
        it must be 3-50 characters, use only ASCII letters and\n\
        numbers, may include single spaces or hyphens between\n\
        words, and must start and end with a letter or number"
    )]
    InvalidPublisher(Box<str>),

    #[error(
        "name `{0}` cannot be used for publishing;
        it must be 3-50 characters, use only ASCII letters and\n\
        numbers, may include single spaces, hyphens, or dots\n\
        between words, and must start and end with a letter or number"
    )]
    InvalidName(Box<str>),

    #[error(
        "version `{version}` cannot be used for publishing: build\n\
        metadata (`+...`) cannot be used for projects in the index"
    )]
    VersionBuildMetadata { version: Box<str> },

    #[error("missing license in project info; it is required for publishing")]
    MissingLicense,

    // Print `ParseError` directly, since its formatting demands a newline before
    #[error(
        "license `{license}` cannot be used for publishing; it must\n\
        be a valid SPDX license expression, but failed to parse:\n{err}"
    )]
    InvalidLicense {
        license: Box<str>,
        err: spdx::error::ParseError,
    },

    #[error("invalid index URL `{url}` for publish: {reason}")]
    InvalidDiscoveryRoot { url: Box<str>, reason: String },

    #[error("invalid api_root URL `{url}` for publish: {reason}")]
    InvalidApiRoot { url: Box<str>, reason: String },

    #[error(
        "no bearer token credentials configured for publish URL `{upload_url}`; \
         set SYSAND_CRED_<X> and SYSAND_CRED_<X>_BEARER_TOKEN with a matching URL pattern"
    )]
    NoPublishBearer { upload_url: Box<str> },

    #[error(
        "multiple bearer token credentials configured for publish URL `{upload_url}`; \
         refine SYSAND_CRED_<X> URL patterns so exactly one bearer token matches ({candidates} candidates found)"
    )]
    AmbiguousPublishBearer {
        upload_url: Box<str>,
        candidates: usize,
    },

    #[error(
        "trusted publishing provider `{}` requires environment variable{} {}",
        provider.name(),
        if variables.len() == 1 { "" } else { "s" },
        variables.join(", ")
    )]
    MissingTrustedPublishingEnvironment {
        provider: TrustedPublishingProvider,
        variables: Box<[&'static str]>,
    },

    #[error(
        "multiple trusted publishing CI environments detected; specify \
         --trusted-publishing=github or --trusted-publishing=gitlab"
    )]
    MultipleTrustedPublishingProviders,

    #[error(
        "trusted publishing provider `github` has invalid ACTIONS_ID_TOKEN_REQUEST_URL: {source}"
    )]
    InvalidGithubOidcRequestUrl { source: url::ParseError },

    #[error("trusted publishing HTTP request failed during {context}: {source:#?}")]
    TrustedPublishingHttp {
        context: &'static str,
        source: reqwest_middleware::Error,
    },

    #[error(
        "trusted publishing provider `{}` failed to acquire OIDC token: HTTP status {status}",
        provider.name()
    )]
    TrustedPublishingProviderHttpStatus {
        provider: TrustedPublishingProvider,
        status: u16,
    },

    #[error("trusted publishing token exchange at `{url}` failed: HTTP status {status}")]
    TrustedPublishingExchangeHttpStatus { url: Box<str>, status: u16 },

    #[error("failed to read {context}: {source:#?}")]
    TrustedPublishingResponseBody {
        context: &'static str,
        source: reqwest::Error,
    },

    #[error("trusted publishing {context} returned malformed response: {source}")]
    MalformedJsonResponse {
        context: &'static str,
        source: serde_json::Error,
    },

    #[error(
        "trusted publishing {context} returned malformed response: missing non-empty `{field}` string"
    )]
    MissingJsonField {
        context: &'static str,
        field: &'static str,
    },

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

// --- Preparation helpers ---

/// Payload ready to POST to the upload endpoint.
#[derive(Debug)]
pub struct PublishPreparation {
    norm_publisher: String,
    norm_name: String,
    version: String,
    // Keep upload payload in `Bytes` so request retries clone cheaply.
    kpar_bytes: Bytes,
    metadata: String,
}

// TODO:
// - warn if unknown fields present in .project.json/.meta.json - this will require
//   a large refactoring to keep track of consistently
//
// All of the publish checks will need to be revisited when we start supporting
// publishing to other indexes, many of them will not apply

/// Reads and validates a `.kpar` file, returning the upload payload and
/// metadata. Does not touch network. Should be called before any network
/// activity.
pub fn prepare_publish_payload(path: &Utf8Path) -> Result<PublishPreparation, PublishError> {
    let file_size = wrapfs::metadata(path).map_err(PublishError::Io)?.len();
    if file_size > MAX_KPAR_PUBLISH_SIZE {
        return Err(PublishError::KparTooLarge {
            size: file_size,
            limit: MAX_KPAR_PUBLISH_SIZE,
        });
    }

    // Open and validate kpar.
    let kpar_project = LocalKParProjectRaw::new_guess_root(path)
        .map_err(|e| PublishError::KparRead(path.as_str().into(), e))?;
    if let Some(p) = kpar_project.project_root_in_archive()
        && !p.as_str().is_empty()
    {
        return Err(PublishError::ProjectNotAtRoot {
            kpar_path: path.as_str().into(),
            root_in_kpar: p.as_str().into(),
        });
    }

    let (info, meta) = kpar_project
        .get_project()
        .map_err(|e| PublishError::KparRead(path.as_str().into(), e))?;

    let info = info.ok_or(PublishError::MissingInfo)?;
    let validated_info = info
        .validate()
        .map_err(|e| PublishError::InfoMetaValidation {
            name: "project",
            source: e,
        })?;
    let meta = meta.ok_or(PublishError::MissingMeta)?;
    // TODO: maybe use parse_sysand_purl() in validate() for usages? This would give better errors
    // than generic IRI parsing
    let validated_meta = meta
        .validate()
        .map_err(|e| PublishError::InfoMetaValidation {
            name: "meta",
            source: e,
        })?;

    // Usages are only `pkg:sysand/` or std libs
    for usage in &info.usage {
        check_usage(usage)?;
    }

    // Publisher
    let publisher = info
        .publisher
        .as_deref()
        .ok_or(PublishError::MissingPublisher)?;
    if !is_valid_unnormalized_publisher(publisher) {
        return Err(PublishError::InvalidPublisher(publisher.into()));
    }
    let normalized_publisher = normalize_field(publisher);

    let name = &info.name;
    if !is_valid_unnormalized_name(name) {
        return Err(PublishError::InvalidName(name.as_str().into()));
    }
    let normalized_name = normalize_field(name);

    let version = &info.version;
    if !validated_info.version.build.is_empty() {
        return Err(PublishError::VersionBuildMetadata {
            version: version.as_str().into(),
        });
    }

    let license = info
        .license
        .as_deref()
        .ok_or(PublishError::MissingLicense)?;
    let license_expr =
        spdx::Expression::parse(license).map_err(|err| PublishError::InvalidLicense {
            license: license.into(),
            err,
        })?;

    let (metamodel, metamodel_kind) = if let Some(m) = &meta.metamodel {
        (m, check_metamodel(m)?)
    } else {
        return Err(PublishError::MissingMetamodel);
    };

    // Get archive file list, all file presence/format checking will be done on that
    let mut archive = kpar_project
        .open_archive()
        .map_err(|e| PublishError::KparRead(path.as_str().into(), e))?;
    // Check for one kind of zip bomb. Other kinds are difficult to check for.
    if archive
        .has_overlapping_files()
        .map_err(|e| PublishError::KparReadZip(path.as_str().into(), e))?
    {
        return Err(PublishError::OverlappingFiles);
    }
    // Bools track whether the file is expected by metadata/our conventions
    let mut kpar_files: HashMap<String, bool> = HashMap::new();
    for i in 0..archive.len() {
        let f = archive
            .by_index_raw(i)
            .map_err(|e| PublishError::KparReadZip(path.as_str().into(), e))?;

        let name = f.name();
        parse_relative_unix_path(name, RelativePathKind::SubDirectory)
            .map_err(PublishError::InvalidPathInArchive)?;

        if f.is_symlink() {
            return Err(PublishError::Symlink { path: name.into() });
        }

        // Directory entries don't contain any contents, so encryption
        // or compression doesn't matter, but extraction can still fail
        // if such metadata is set
        if f.encrypted() {
            return Err(PublishError::Encrypted { path: name.into() });
        }
        if !f.is_dir() {
            if f.compression() != zip::CompressionMethod::Deflated {
                return Err(PublishError::UnsupportedCompression {
                    path: name.into(),
                    comp: f.compression(),
                });
            }
            // Check all exec bits for files; exec bit for dirs means dir can be opened
            if let Some(mode) = f.unix_mode()
                && (mode & 0o111) != 0
            {
                return Err(PublishError::ExecInArchive { path: name.into() });
            }

            // Ignore directory entries, as we don't have any use for them
            kpar_files.insert(name.to_owned(), false);
        } else if f.compression() != zip::CompressionMethod::Stored {
            return Err(PublishError::CompressedDirEntry {
                path: name.into(),
                comp: f.compression(),
            });
        }
    }

    for stem in license_file_stems(&license_expr) {
        let license_path = format!("LICENSES/{stem}.txt");
        match kpar_files.get_mut(license_path.as_str()) {
            Some(v) => *v = true,
            None => {
                return Err(PublishError::MissingLicenseFile {
                    path: license_path.into_boxed_str(),
                    license: license.into(),
                });
            }
        };
    }

    let Some(file_checksums) = validated_meta.checksum else {
        return Err(PublishError::MissingChecksum);
    };
    if file_checksums.is_empty() {
        return Err(PublishError::EmptyChecksum);
    }
    if validated_meta.index.is_empty() {
        log::warn!(
            "project doesn't list any symbols as exported (empty `index`
            {0:>8} field in `.meta.json`);\n\
            {0:>8} it's unlikely to be useful if no symbols are exported\n\
            {0:>8} (did you forget to include source files?)",
            ' '
        );
    }

    // Reverse of `meta.index`: instead of symbol -> file, this is file -> symbols
    let mut symbols_for_files: HashMap<_, Vec<String>> = HashMap::new();

    for (symbol, src_file) in validated_meta.index {
        // `meta.checksum` must reference a superset of files mentioned by `meta.index`
        // TODO: what about detecting duplicate exports? With our current design this is by
        // construction impossible.
        // Should probably change InterchangeProjectMetadataRaw to have Vec for checksum and
        // index, instead of HashMap, which will silently discard duplicates; and then fail
        // on `validate()`, as such metadata is always incorrect
        if !file_checksums.contains_key(&src_file) {
            return Err(PublishError::MissingChecksumForFile {
                path: src_file.as_str().into(),
            });
        }
        match symbols_for_files.entry(src_file) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(symbol);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![symbol]);
            }
        }
    }

    for (src_file, file_checksum) in file_checksums {
        if !src_file.as_str().ends_with(metamodel_kind.file_ext()) {
            return Err(PublishError::IncorrectFileFormat {
                path: src_file.as_str().into(),
                metamodel_iri: metamodel.as_str().into(),
                metamodel: metamodel_kind,
            });
        }

        if file_checksum.algorithm != KerMlChecksumAlg::Sha256 {
            return Err(PublishError::UnsupportedFileChecksumType {
                path: src_file.as_str().into(),
                alg: file_checksum.algorithm,
            });
        }

        match kpar_files.get_mut(src_file.as_str()) {
            Some(v) => {
                *v = true;

                let mut file =
                    archive
                        .by_path(src_file.as_str())
                        .map_err(|e| PublishError::KparFileRead {
                            path: src_file.as_str().into(),
                            source: e,
                        })?;
                let mut source = String::new();
                file.read_to_string(&mut source)
                    .map_err(|e| PublishError::KparFileReadIo {
                        path: src_file.as_str().into(),
                        source: e,
                    })?;
                let actual_checksum = sha256_lowercase_hex(&source);
                if !actual_checksum.eq_ignore_ascii_case(&file_checksum.value) {
                    return Err(PublishError::IncorrectFileChecksum {
                        path: src_file.as_str().into(),
                        expected: file_checksum.value.into_boxed_str(),
                        actual: actual_checksum.into_boxed_str(),
                    });
                }

                let actual_symbols =
                    extract_symbols(&src_file, &source, Some(metamodel_kind.lang_kind()))
                        .map_err(|e| PublishError::IndexFail { source: e })?;
                // Actual symbols must be a superset of recorded
                if let Some(symbols) = symbols_for_files.get(&src_file) {
                    for s in symbols {
                        if !actual_symbols.contains(s) {
                            return Err(PublishError::NonexistentSymbolExported {
                                path: src_file.as_str().into(),
                                symbol: s.as_str().into(),
                                actual_symbols: actual_symbols.into_boxed_slice(),
                            });
                        }
                    }
                } else if !actual_symbols.is_empty() {
                    // It is valid for a file to not export any symbols
                    log::warn!(
                        "project file `{src_file}` exports symbols {actual_symbols:?},\n\
                    {0:>8} but they are not mentioned in `.meta.json`; this is valid,\n\
                    {0:>8} but likely to be an error",
                        ' '
                    );
                }
            }
            None => {
                return Err(PublishError::MissingFile {
                    path: src_file.as_str().into(),
                });
            }
        };
    }

    match kpar_files.get_mut("README.md") {
        Some(v) => *v = true,
        None => log::warn!(
            "KPAR does not contain a readme file README.md; it is
            {0:>8} recommended to provide it to serve as introduction to users",
            ' '
        ),
    }
    match kpar_files.get_mut("CHANGELOG.md") {
        Some(v) => *v = true,
        None => log::warn!(
            "KPAR does not contain a changelog file CHANGELOG.md;\n\
            {0:>8} it is recommended to provide it to inform users of\n\
            {0:>8} the changes between versions",
            ' '
        ),
    }
    kpar_files.remove_entry(".project.json").unwrap();
    kpar_files.remove_entry(".meta.json").unwrap();

    for (path, expected) in kpar_files {
        if !expected {
            return Err(PublishError::UnexpectedFile { path: path.into() });
        }
    }

    let kpar_bytes = wrapfs::read(path).map_err(PublishError::Io)?;
    let sha256_digest = sha256_lowercase_hex(&kpar_bytes);
    let metadata = serde_json::json!({
        "normalized_publisher": normalized_publisher,
        "normalized_name": normalized_name,
        "version": version,
        "license": license,
        "kpar_sha256_digest": sha256_digest,
    })
    .to_string();

    Ok(PublishPreparation {
        norm_publisher: normalized_publisher,
        norm_name: normalized_name,
        version: version.to_owned(),
        metadata,
        kpar_bytes: Bytes::from(kpar_bytes),
    })
}

/// `Ok(_)` - valid SysML/KerML metamodel
/// `Err` - invalid SysML/KerML or other
fn check_metamodel(metamodel: &str) -> Result<AllowedMetamodelKind, PublishError> {
    for (prefix, kind) in [
        (SYSML_METAMODEL_PREFIX, AllowedMetamodelKind::SysML),
        (KERML_METAMODEL_PREFIX, AllowedMetamodelKind::KerML),
    ] {
        if let Some(metamodel_version) = metamodel.strip_prefix(prefix) {
            if is_valid_metamodel_version(metamodel_version) {
                return Ok(kind);
            } else {
                return Err(PublishError::InvalidMetamodelVersion {
                    metamodel: metamodel.into(),
                    version: metamodel_version.into(),
                });
            }
        }
    }
    Err(PublishError::UnsupportedMetamodel {
        metamodel: metamodel.to_owned(),
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

const KERML_STD_LIB_SUFFIXES: [&str; 3] = [
    "/Semantic-Library.kpar",
    "/Data-Type-Library.kpar",
    "/Function-Library.kpar",
];

const SYSML_STD_LIB_SUFFIXES: [&str; 7] = [
    "/Systems-Library.kpar",
    "/Analysis-Domain-Library.kpar",
    "/Cause-and-Effect-Domain-Library.kpar",
    "/Geometry-Domain-Library.kpar",
    "/Metadata-Domain-Library.kpar",
    "/Quantities-and-Units-Domain-Library.kpar",
    "/Requirement-Derivation-Domain-Library.kpar",
];

/// A usage can be either `pkg:sysand/` or an std lib
// TODO: be case insensitive
fn check_usage(usage: &InterchangeProjectUsageRaw) -> Result<(), PublishError> {
    if check_std_libs(
        usage,
        &SYSML_STD_LIB_SUFFIXES,
        "https://www.omg.org/spec/SysML/",
    )? || check_std_libs(
        usage,
        &KERML_STD_LIB_SUFFIXES,
        "https://www.omg.org/spec/KerML/",
    )? {
        return Ok(());
    }

    match usage {
        InterchangeProjectUsageRaw::Resource { resource, .. } => {
            match parse_sysand_purl(resource) {
                Ok(Some(_)) => Ok(()),
                Ok(None) => Err(PublishError::DisallowedUsage {
                    name: resource.as_str().into(),
                }),
                Err(e) => Err(PublishError::InvalidPurl {
                    name: resource.as_str().into(),
                    source: e,
                }),
            }
        }
    }
}

/// `Ok(true)` - matched
/// `Ok(false)` - not matched
/// `Err` - matched, but incorrect
fn check_std_libs(
    usage: &InterchangeProjectUsageRaw,
    lib_names: &[&str],
    prefix: &str,
) -> Result<bool, PublishError> {
    match usage {
        InterchangeProjectUsageRaw::Resource {
            resource,
            version_constraint,
        } => {
            if let Some(stripped) = resource.strip_prefix(prefix) {
                for s in lib_names {
                    if let Some(metamodel_version) = stripped.strip_suffix(s) {
                        if is_valid_metamodel_version(metamodel_version) {
                            if let Some(vc) = version_constraint.as_deref() {
                                return Err(PublishError::StdWithVersionConstraint {
                                    name: resource.as_str().into(),
                                    vc: vc.into(),
                                });
                            }
                            return Ok(true);
                        } else {
                            return Err(PublishError::InvalidStdLibVersion {
                                name: resource.as_str().into(),
                                std_version: metamodel_version.into(),
                            });
                        }
                    }
                }
                return Err(PublishError::UnknownStdLib {
                    name: resource.as_str().into(),
                });
            }
            Ok(false)
        }
    }
}

/// Check that `v` is a number of the form `20yymmxx`, where `yy` and `xx` are pairs
/// of digits, and `1 <= mm <= 12`
fn is_valid_metamodel_version(v: &str) -> bool {
    let first_release = "20250201";
    if v.len() != first_release.len() || !v.starts_with("20") {
        return false;
    }

    let number: u32 = match v.parse() {
        Ok(n) => n,
        Err(_) => return false,
    };

    // Year is already constrained to start with 20,
    // and release format is unspecified number
    let month = (number / 100) % 100;

    (1..=12).contains(&month)
}

/// All metamodels allowed by the index
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AllowedMetamodelKind {
    SysML,
    KerML,
}

impl AllowedMetamodelKind {
    fn file_ext(&self) -> &'static str {
        match self {
            AllowedMetamodelKind::SysML => ".sysml",
            AllowedMetamodelKind::KerML => ".kerml",
        }
    }

    fn lang(&self) -> &'static str {
        match self {
            AllowedMetamodelKind::SysML => "SysMLv2",
            AllowedMetamodelKind::KerML => "KerML",
        }
    }

    fn lang_kind(&self) -> Language {
        match self {
            AllowedMetamodelKind::SysML => Language::SysML,
            AllowedMetamodelKind::KerML => Language::KerML,
        }
    }
}

#[cfg(test)]
#[path = "./publish_tests.rs"]
mod tests;
