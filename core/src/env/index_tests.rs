// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Tests for the index-protocol HTTP client.
//!
//! Cross-cutting protocol assumptions these tests exercise (hoisted here
//! so per-test comments can focus on what each test adds):
//!
//! - **404 handling splits by document.** `index.json` and the
//!   per-version `.project.json` / `.meta.json` 404s remain hard
//!   errors (empty-but-live collections are served as 200 with an
//!   empty payload). `versions.json` 404 means "project not in this
//!   index" (§8): `versions_async` yields an empty stream so a
//!   resolver chain can try another source, and `get_project_async`
//!   surfaces a distinct `ProjectNotInIndex` error so direct callers
//!   can tell "not here" apart from a transport failure.
//! - **`versions.json` entries MUST be in descending semver
//!   precedence** and unique. The client validates ordering at
//!   ingest (not lexically) and does not re-sort; downstream code
//!   relies on newest-first.
//! - **SHA-256 is the only supported digest algorithm**, and the
//!   canonical project digest MUST be computable from `.project.json`
//!   and `.meta.json` alone. Digest fields are lowercase
//!   `sha256:<64-hex>` on the wire.
//! - **The server is authoritative for textual fields.** The client
//!   does not diff `info.version` / `info.usage` against the
//!   `versions.json` entry — the `project_digest` is the integrity
//!   check.
//! - **`project_digest` MUST be verified before exposing
//!   `.project.json` or `.meta.json` to callers.** A mismatch surfaces
//!   as `AdvertisedDigestDrift`; metadata that would require source reads
//!   to canonicalize is rejected before either document is released.
//! - **Redirects are followed on every resource** (index docs and
//!   the discovery fetch alike); `reqwest`'s default policy applies.
//! - **Unknown JSON fields are silently ignored** for forward
//!   compatibility.

use std::sync::Arc;

use crate::{
    auth::Unauthenticated,
    context::ProjectContext,
    env::{ReadEnvironment, ReadEnvironmentAsync, discovery::ResolvedEndpoints},
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, project_hash_raw},
    project::{
        InlineProjectDigest, ProjectRead, canonical_project_digest_inline,
        index_entry::IndexEntryProjectError, reqwest_kpar_download::ReqwestKparDownloadedError,
    },
    purl::PKG_SYSAND_PREFIX,
    resolve::net_utils::create_reqwest_client,
};

// Re-exports so that `super::X` paths inside sub-modules (which refer to this
// `tests` module) continue to resolve to names from the parent `index` module.
use super::{HttpFetchError, IndexEnvironmentError};

/// Placeholder sha256 value acceptable by `parse_sha256_digest` — used in
/// tests that exercise flow but don't care about the specific digest bytes.
/// All-`a`s so it's visibly distinct from real-digest tests below.
const FILLER_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// Build a `pkg:sysand/<suffix>` IRI from the constant prefix. Tests rely on
/// this so the literal scheme prefix lives in exactly one place
/// (`PKG_SYSAND_PREFIX` in `purl.rs`).
fn purl(suffix: &str) -> String {
    format!("{PKG_SYSAND_PREFIX}{suffix}")
}

/// Render a minimal-but-valid `versions.json` body for the given (version,
/// usage) pairs. The three required artifact fields are populated with
/// placeholder values; tests that need specific digest or size semantics
/// construct the body inline instead.
fn versions_json_body<const N: usize>(entries: [(&str, &str); N]) -> String {
    let parts: Vec<String> = entries
        .iter()
        .map(|(version, usage)| {
            format!(
                r#"{{"version":"{version}","usage":{usage},"project_digest":"{FILLER_DIGEST}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}"}}"#
            )
        })
        .collect();
    format!(r#"{{"versions":[{}]}}"#, parts.join(","))
}

fn versions_json_body_with_project_digest<const N: usize>(
    entries: [(&str, &str, &str); N],
) -> String {
    let parts: Vec<String> = entries
        .iter()
        .map(|(version, usage, project_digest)| {
            format!(
                r#"{{"version":"{version}","usage":{usage},"project_digest":"{project_digest}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}"}}"#
            )
        })
        .collect();
    format!(r#"{{"versions":[{}]}}"#, parts.join(","))
}

/// Render a minimal `.project.json` body for the given fields. `usage` is
/// inlined as a raw JSON fragment so callers can pass `"[]"` or
/// `r#"[{"resource":"..."}]"#` without escaping.
fn project_json_body(name: &str, publisher: Option<&str>, version: &str, usage: &str) -> String {
    match publisher {
        Some(p) => format!(
            r#"{{"name":"{name}","publisher":"{p}","version":"{version}","usage":{usage}}}"#
        ),
        None => format!(r#"{{"name":"{name}","version":"{version}","usage":{usage}}}"#),
    }
}

/// Render a minimal-but-valid `.meta.json` body. The fixed timestamp keeps
/// any test that hashes the body reproducible.
fn meta_json_body() -> &'static str {
    r#"{"index":{},"created":"2026-01-01T00:00:00.000000000Z"}"#
}

fn project_digest(info_json: &str, meta_json: &str) -> Result<String, Box<dyn std::error::Error>> {
    let info: InterchangeProjectInfoRaw = serde_json::from_str(info_json)?;
    let meta: InterchangeProjectMetadataRaw = serde_json::from_str(meta_json)?;
    Ok(format!("sha256:{:x}", project_hash_raw(&info, &meta)))
}

/// Compute the canonical project digest — matches what the server would
/// advertise in `versions.json`'s `project_digest`. Equivalent to
/// `project_digest` when `meta` has no checksum entries or only lowercase
/// SHA256 entries, but differs when entries require canonicalization
/// (mixed-case SHA256 hex values).
fn canonical_project_digest(
    info_json: &str,
    meta_json: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let info: InterchangeProjectInfoRaw = serde_json::from_str(info_json)?;
    let meta: InterchangeProjectMetadataRaw = serde_json::from_str(meta_json)?;
    let InlineProjectDigest::Computed(hash) = canonical_project_digest_inline(&info, &meta) else {
        panic!("canonical digest should be computable inline for this fixture");
    };
    Ok(format!("sha256:{:x}", hash))
}

fn make_runtime() -> Result<Arc<tokio::runtime::Runtime>, Box<dyn std::error::Error>> {
    Ok(Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    ))
}

/// Build an unauthenticated async index environment whose discovery root
/// is `base_url`. `base_url` serves both as the discovery root and the
/// index/api root for tests that don't cover the discovery flow — those
/// tests seed the `resolved` cell directly so no discovery fetch is
/// issued against the mock server.
fn test_env_async(
    base_url: &str,
) -> Result<super::IndexEnvironmentAsync<Unauthenticated>, Box<dyn std::error::Error>> {
    let base = url::Url::parse(base_url)?;
    // Use a flat topology (index_root = api_root = discovery root).
    // Tests that specifically cover discovery construct their own env
    // via `test_env_sync_discovery`, which goes through the real
    // `fetch_index_config` against a mock server.
    let endpoints = ResolvedEndpoints::flat(with_trailing_slash(base));
    Ok(super::IndexEnvironmentAsync::new(
        create_reqwest_client()?,
        Arc::new(Unauthenticated {}),
        endpoints,
    ))
}

/// Helper: return `url` with a guaranteed trailing slash on its path.
/// Kept here (rather than pulled from `super::discovery::with_trailing_slash`
/// which is private) so test-side URL construction matches what the
/// discovery module produces.
fn with_trailing_slash(mut url: url::Url) -> url::Url {
    if url.path().is_empty() {
        url.set_path("/");
    } else if !url.path().ends_with('/') {
        let new_path = format!("{}/", url.path());
        url.set_path(&new_path);
    }
    url
}

/// Shorthand: resolve the endpoints on a test env. Tests that don't
/// cover discovery specifically use this rather than calling `endpoints`
/// on the env repeatedly.
fn test_endpoints(env: &super::IndexEnvironmentAsync<Unauthenticated>) -> &ResolvedEndpoints {
    env.endpoints
        .get()
        .expect("test_env_async initializes resolved endpoints")
}

/// Build a sync-facing env after resolving discovery against the mock
/// server. This exercises the real `sysand-index-config.json` fetch, but
/// does it eagerly so discovery errors surface during test setup.
fn test_env_sync_discovery(
    server: &mockito::Server,
) -> Result<
    crate::env::AsSyncEnvironmentTokio<super::IndexEnvironmentAsync<Unauthenticated>>,
    Box<dyn std::error::Error>,
> {
    let base = url::Url::parse(&server.url())?;
    let runtime = make_runtime()?;
    let client = create_reqwest_client()?;
    let auth = Arc::new(Unauthenticated {});
    let endpoints = runtime.block_on(crate::env::discovery::fetch_index_config(
        &client, &*auth, &base,
    ))?;
    let env = super::IndexEnvironmentAsync::new(client, auth, endpoints);
    Ok(env.to_tokio_sync(runtime))
}

/// Build a sync-facing unauthenticated index environment rooted at `server`,
/// with a runtime owned by the test. This is the shape most tests want — a
/// mock `server` plus a single blocking handle to call `.get_project(...)` /
/// `.uris()` / `.versions(...)` against it.
fn test_env_sync(
    server: &mockito::Server,
) -> Result<
    crate::env::AsSyncEnvironmentTokio<super::IndexEnvironmentAsync<Unauthenticated>>,
    Box<dyn std::error::Error>,
> {
    Ok(test_env_async(&server.url())?.to_tokio_sync(make_runtime()?))
}

/// Register a mock for `{method} {path}` asserting it must never be called.
/// Body/status are immaterial — any match would fail `mock.assert()`.
fn expect_untouched(server: &mut mockito::Server, method: &str, path: &str) -> mockito::Mock {
    server.mock(method, path).expect(0).create()
}

/// Register a `GET {path}` mock returning `200 application/json` with `body`,
/// requiring exactly one hit. Covers the dominant mock shape in this file;
/// bespoke shapes (non-200, non-JSON, exact `expect(n)`, or permissive mocks
/// with no call-count expectation) use `server.mock(...)` directly.
fn mock_json_get(
    server: &mut mockito::Server,
    path: &str,
    body: impl Into<String>,
) -> mockito::Mock {
    mock_json_get_count(server, path, body, 1)
}

fn mock_json_get_count(
    server: &mut mockito::Server,
    path: &str,
    body: impl Into<String>,
    expected_count: usize,
) -> mockito::Mock {
    server
        .mock("GET", path)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body.into())
        .expect(expected_count)
        .create()
}

/// Build a minimal kpar (ZIP) archive carrying `.project.json`,
/// `.meta.json`, and a single source file at the archive root, returning
/// the archive bytes alongside the exact info/meta JSON strings written
/// into it. Tests that also mock the per-version `.project.json` /
/// `.meta.json` endpoints reuse those strings so the index-served content
/// matches the in-archive content — the only deliberate drift remains in
/// the advertised `project_digest`.
fn build_minimal_kpar(
    name: &str,
    version: &str,
    src_path: &str,
    src_body: &str,
) -> (Vec<u8>, String, &'static str) {
    use std::io::Write as _;
    let info_json = format!(r#"{{"name":"{name}","version":"{version}","usage":[]}}"#);
    let meta_json: &'static str = r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#;
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        zip.start_file(".project.json", options).unwrap();
        zip.write_all(info_json.as_bytes()).unwrap();
        zip.start_file(".meta.json", options).unwrap();
        zip.write_all(meta_json.as_bytes()).unwrap();
        zip.start_file(src_path, options).unwrap();
        zip.write_all(src_body.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    (buf, info_json, meta_json)
}

mod uris {
    use super::*;

    #[test]
    fn uri_examples() -> Result<(), Box<dyn std::error::Error>> {
        let env = test_env_async("https://www.example.com/index/")?;
        let endpoints = test_endpoints(&env);

        assert_eq!(
            endpoints.index_url()?.to_string(),
            "https://www.example.com/index/index.json"
        );

        // pkg:sysand/<publisher>/<name> routes under publisher/name/
        assert_eq!(
            endpoints
                .kpar_url(purl("admin/proj0"), "0.3.0")?
                .to_string(),
            "https://www.example.com/index/admin/proj0/0.3.0/project.kpar"
        );

        // Non-pkg:sysand IRIs go under _iri/<sha256(normalized_iri)>/
        assert_eq!(
            endpoints.kpar_url("urn:kpar:b", "1.0.0")?.to_string(),
            "https://www.example.com/index/_iri/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/1.0.0/project.kpar"
        );

        // Per-version `.project.json` lives in the same version directory; this
        // also exercises the `version_dir_url` trailing-slash invariant.
        assert_eq!(
            endpoints
                .project_json_url(purl("admin/proj0"), "0.3.0")?
                .to_string(),
            "https://www.example.com/index/admin/proj0/0.3.0/.project.json"
        );

        Ok(())
    }

    #[test]
    fn invalid_or_non_normalized_sysand_purl_rejected_loudly()
    -> Result<(), Box<dyn std::error::Error>> {
        // The `pkg:sysand/` prefix is strong intent; a malformed or
        // non-normalized variant must reject as `MalformedSysandPurl`
        // rather than silently reroute to `_iri/<sha256>/` (which
        // would mask typos, casing, traversal attempts, and wrong
        // segment counts as opaque "not found"s).
        let env = test_env_async("https://www.example.com/index/")?;

        for iri in [
            // traversal / URL-syntax attacks
            &purl("../attacker"),
            &purl("..%2Fattacker/proj"),
            &purl("./proj"),
            &purl(".hidden/proj"),
            &purl("pub/.hidden"),
            // non-ASCII
            &purl("Åcme/proj"),
            // valid but not normalized (uppercase, spaces)
            &purl("Admin/proj0"),
            &purl("admin/My Project"),
            // too short (min 3 chars)
            &purl("ab/proj0"),
            // control characters
            &purl("pub\t/proj"),
            // URL-syntax characters
            &purl("pub#frag/proj"),
            &purl("pub?q/proj"),
            // wrong segment count
            &purl("a/b/c"),
            &purl("a/"),
            &purl(""),
        ] {
            let err = test_endpoints(&env)
                .kpar_url(iri, "1.0.0")
                .expect_err(&format!(
                    "expected `{iri}` to be rejected as malformed pkg:sysand"
                ));
            assert!(
                matches!(
                    err,
                    super::IndexEnvironmentError::MalformedSysandPurl { .. }
                ),
                "expected MalformedSysandPurl for `{iri}`, got {err:?}"
            );
        }

        Ok(())
    }

    #[test]
    fn non_normalized_sysand_purl_error_suggests_normalized_form()
    -> Result<(), Box<dyn std::error::Error>> {
        // The error message for the "valid but not normalized" case must include
        // the suggested normalized IRI — that's what makes the error actionable
        // (otherwise the user just sees "rejected" with no path forward).
        let env = test_env_async("https://www.example.com/index/")?;
        let err = test_endpoints(&env)
            .kpar_url(purl("Acme Labs/My.Project"), "1.0.0")
            .expect_err("non-normalized pkg:sysand must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains(&purl("acme-labs/my.project")),
            "error message `{msg}` must surface the suggested normalized IRI"
        );
        Ok(())
    }

    #[test]
    fn base_url_without_trailing_slash() -> Result<(), Box<dyn std::error::Error>> {
        let env = test_env_async("https://www.example.com/index")?;
        let endpoints = test_endpoints(&env);

        assert_eq!(
            endpoints.index_url()?.to_string(),
            "https://www.example.com/index/index.json"
        );
        assert_eq!(
            endpoints
                .kpar_url(purl("admin/proj0"), "0.3.0")?
                .to_string(),
            "https://www.example.com/index/admin/proj0/0.3.0/project.kpar"
        );

        Ok(())
    }

    #[test]
    fn uris_from_index_json_accepts_omitted_and_explicit_available_status()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let index_mock = mock_json_get(
            &mut server,
            "/index.json",
            format!(
                r#"{{
                "projects": [
                    {{ "iri": "{PKG_SYSAND_PREFIX}admin/proj0" }},
                    {{ "iri": "urn:kpar:b", "status": "available" }}
                ]
            }}"#
            ),
        );

        let uris: Result<Vec<_>, _> = env.uris()?.collect();
        let uris = uris?;

        assert_eq!(uris.len(), 2);
        assert!(uris.contains(&purl("admin/proj0")));
        assert!(uris.contains(&"urn:kpar:b".to_string()));

        index_mock.assert();

        Ok(())
    }

    #[test]
    fn uris_from_index_json_filters_removed_projects() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let index_mock = mock_json_get(
            &mut server,
            "/index.json",
            format!(
                r#"{{
                "projects": [
                    {{ "iri": "{PKG_SYSAND_PREFIX}admin/proj0", "status": "removed" }},
                    {{ "iri": "urn:kpar:b" }}
                ]
            }}"#
            ),
        );

        let uris: Vec<_> = env.uris()?.collect::<Result<_, _>>()?;

        assert_eq!(uris, vec!["urn:kpar:b"]);

        index_mock.assert();

        Ok(())
    }

    #[test]
    fn index_json_rejects_yanked_project_status() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let index_mock = mock_json_get(
            &mut server,
            "/index.json",
            format!(
                r#"{{
                "projects": [
                    {{ "iri": "{PKG_SYSAND_PREFIX}admin/proj0", "status": "yanked" }}
                ]
            }}"#
            ),
        );

        let err = env
            .uris()
            .expect_err("`yanked` is not valid for index.json project status");
        match err {
            super::IndexEnvironmentError::Fetch(super::HttpFetchError::JsonParse {
                url, ..
            }) => assert!(url.contains("/index.json"), "url carried: {url}"),
            other => panic!("expected Fetch(JsonParse), got {other:?}"),
        }
        index_mock.assert();

        Ok(())
    }

    #[test]
    fn missing_index_is_hard_error() -> Result<(), Box<dyn std::error::Error>> {
        // `index.json` is the document that identifies a URL as a sysand
        // index; a 404 means "this URL is not a sysand index", so a
        // misconfigured base URL surfaces clearly instead of masquerading
        // as an empty index.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let index_mock = server
            .mock("GET", "/index.json")
            .with_status(404)
            .with_body("not found")
            .create();

        let err = env
            .uris()
            .expect_err("missing index.json must error, not be treated as empty");
        match err {
            super::IndexEnvironmentError::Fetch(super::HttpFetchError::BadHttpStatus {
                url,
                status,
            }) => {
                assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
                assert!(url.contains("/index.json"), "url carried: {url}");
            }
            other => panic!("expected Fetch(BadHttpStatus 404), got {other:?}"),
        }
        index_mock.assert();

        Ok(())
    }

    #[test]
    fn empty_but_live_index_yields_no_uris() -> Result<(), Box<dyn std::error::Error>> {
        // The 200-with-empty-list path — distinct from the 404-is-hard-error
        // path above.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let index_mock = server
            .mock("GET", "/index.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"projects": []}"#)
            .create();

        let uris: Result<Vec<_>, _> = env.uris()?.collect();
        assert!(uris?.is_empty());
        index_mock.assert();

        Ok(())
    }

    #[test]
    fn server_error_surfaces() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let index_mock = server
            .mock("GET", "/index.json")
            .with_status(500)
            .with_body("server error")
            .create();

        assert!(env.uris().is_err());
        index_mock.assert();

        Ok(())
    }

    #[test]
    fn malformed_index_json_surfaces_parse_error() -> Result<(), Box<dyn std::error::Error>> {
        // Misconfigured reverse proxy serves an HTML error page with 200 OK:
        // must surface as `JsonParse` with the URL preserved, not silently
        // treat the index as empty.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let index_mock = server
            .mock("GET", "/index.json")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body>not json</body></html>")
            .create();

        let err = env.uris().expect_err("malformed JSON must error");
        match err {
            super::IndexEnvironmentError::Fetch(super::HttpFetchError::JsonParse {
                url, ..
            }) => {
                assert!(url.contains("/index.json"), "url carried: {url}");
            }
            other => panic!("expected Fetch(JsonParse), got {other:?}"),
        }
        index_mock.assert();

        Ok(())
    }
}

/// Tests for `versions.json` ingest, validation, and streaming.
///
/// Document-level rules exercised here (see module-level doc for the
/// cross-cutting ones):
/// - `version` MUST parse as semver and MUST NOT carry `+build` metadata.
/// - Duplicates are rejected; the five per-entry fields
///   (`version`, `usage`, `project_digest`, `kpar_size`, `kpar_digest`)
///   are all required.
/// - Wire order is preserved verbatim; ascending order (or a prerelease
///   appearing before its release) rejects the whole document.
mod versions {
    use super::*;

    #[test]
    fn versions_from_versions_json() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let pkg_versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("0.3.0", "[]"), ("0.2.0", "[]")]),
        );

        let iri_versions_mock = mock_json_get(
            &mut server,
            "/_iri/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/versions.json",
            versions_json_body([("1.0.0", "[]")]),
        );

        let pkg_versions: Result<Vec<_>, _> = env.versions(purl("admin/proj0"))?.collect();
        let pkg_versions = pkg_versions?;

        assert_eq!(pkg_versions.len(), 2);
        assert!(pkg_versions.contains(&"0.3.0".to_string()));
        assert!(pkg_versions.contains(&"0.2.0".to_string()));

        let iri_versions: Result<Vec<_>, _> = env.versions("urn:kpar:b")?.collect();
        assert_eq!(iri_versions?, vec!["1.0.0"]);

        pkg_versions_mock.assert();
        iri_versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_async_filters_retired_entries() -> Result<(), Box<dyn std::error::Error>> {
        // §8 `status` field + §12 "MUST NOT select non-available for a
        // new resolution". A mixed-status fixture verifies two things
        // in one document:
        //   1. Wire-level parsing accepts every `status` value plus
        //      the omitted default (§14 forward compat + §8 SHOULD-omit
        //      convention for `available`).
        //   2. The resolver-facing `versions()` stream exposes only
        //      `available` entries; `yanked` and `removed` are filtered
        //      so solve/lock can't pick them.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let body = format!(
            r#"{{"versions":[
                {{"version":"4.0.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}","status":"available"}},
                {{"version":"3.0.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}","status":"yanked"}},
                {{"version":"2.0.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}","status":"removed"}},
                {{"version":"1.0.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}"}}
            ]}}"#
        );

        let versions_mock = mock_json_get(&mut server, "/admin/proj0/versions.json", body);

        let versions: Vec<_> = env
            .versions(purl("admin/proj0"))?
            .collect::<Result<_, _>>()?;
        // Explicit `"available"` and omitted `status` both survive the
        // filter; `yanked` and `removed` are dropped.
        assert_eq!(versions, vec!["4.0.0", "1.0.0"]);

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_preserves_server_order() -> Result<(), Box<dyn std::error::Error>> {
        // Semver-tricky fixture makes pass-through visible: a lexicographic-
        // sort regression would reorder `10.0.0` before `10.0.0-beta.1`.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("10.0.0", "[]"), ("10.0.0-beta.1", "[]"), ("2.0.0", "[]")]),
        );

        let versions: Vec<_> = env
            .versions(purl("admin/proj0"))?
            .collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["10.0.0", "10.0.0-beta.1", "2.0.0"]);

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_rejects_ascending_order() -> Result<(), Box<dyn std::error::Error>> {
        // Plain ascending wire order is rejected loudly.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("1.0.0", "[]"), ("2.0.0", "[]"), ("10.0.0", "[]")]),
        );

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("ascending order must be rejected as a protocol violation");
        match err {
            super::IndexEnvironmentError::VersionsOutOfOrder { prev, curr, .. } => {
                assert_eq!(prev, "1.0.0");
                assert_eq!(curr, "2.0.0");
            }
            other => panic!("expected VersionsOutOfOrder, got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_rejects_prerelease_before_release() -> Result<(), Box<dyn std::error::Error>> {
        // Guard against a lexical-vs-semver regression: `10.0.0-beta.1`
        // sorts before `10.0.0` by semver precedence, so emitting the
        // prerelease first must be rejected even though lexical order
        // would accept it.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("10.0.0-beta.1", "[]"), ("10.0.0", "[]")]),
        );

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("prerelease before release must be rejected");
        match err {
            super::IndexEnvironmentError::VersionsOutOfOrder { prev, curr, .. } => {
                assert_eq!(prev, "10.0.0-beta.1");
                assert_eq!(curr, "10.0.0");
            }
            other => panic!("expected VersionsOutOfOrder, got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_rejects_build_metadata() -> Result<(), Box<dyn std::error::Error>> {
        // `semver::Version` is lenient on `+build`, so the rejection must
        // be explicit.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("1.2.3+build.42", "[]")]),
        );

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("build metadata must be rejected as a protocol violation");
        match err {
            super::IndexEnvironmentError::VersionHasBuildMetadata { value, .. } => {
                assert_eq!(value, "1.2.3+build.42");
            }
            other => panic!("expected VersionHasBuildMetadata, got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_accepts_prerelease_without_build() -> Result<(), Box<dyn std::error::Error>> {
        // Prerelease identifiers (`-rc.1`) are permitted; the
        // build-metadata rejection must not catch them by mistake.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("1.2.3-rc.1", "[]")]),
        );

        let versions: Vec<_> = env
            .versions(purl("admin/proj0"))?
            .collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["1.2.3-rc.1"]);

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_non_semver_version_errors() -> Result<(), Box<dyn std::error::Error>> {
        // Without a parseable semver the client cannot order entries —
        // reject the whole document.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("not-a-version", "[]")]),
        );

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("non-semver version must surface as a protocol error");
        match err {
            super::IndexEnvironmentError::InvalidSemverVersion { value, .. } => {
                assert_eq!(value, "not-a-version");
            }
            other => panic!("expected InvalidSemverVersion, got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn missing_versions_json_surfaces_as_empty_at_resolver_boundary()
    -> Result<(), Box<dyn std::error::Error>> {
        // §8 — a 404 on `versions.json` means the project is not in
        // this index. `versions_async` yields an empty stream so a
        // resolver chain falls through to the next source; non-404
        // errors still propagate as hard errors.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = server
            .mock("GET", "/nope/nope/versions.json")
            .with_status(404)
            .with_body("not found")
            .create();

        let versions: Vec<_> = env.versions(purl("nope/nope"))?.collect::<Result<_, _>>()?;
        assert!(
            versions.is_empty(),
            "versions.json 404 must yield an empty stream (project not in this index); \
             got {versions:?}"
        );

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_forbidden_is_hard_error() -> Result<(), Box<dyn std::error::Error>> {
        // The §8 404 downgrade is deliberately narrow. A direct 403 on
        // root `versions.json` is a hard transport/auth failure, not
        // "project absent from this index".
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = server
            .mock("GET", "/admin/proj0/versions.json")
            .with_status(403)
            .with_body("forbidden")
            .create();

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("403 on root versions.json must be a hard error");
        match err {
            super::IndexEnvironmentError::Fetch(super::HttpFetchError::BadHttpStatus {
                status,
                ..
            }) => {
                assert_eq!(status, reqwest::StatusCode::FORBIDDEN);
            }
            other => panic!("expected BadHttpStatus(403), got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn missing_required_field_errors() -> Result<(), Box<dyn std::error::Error>> {
        // An entry omitting any required field (here `kpar_digest`)
        // rejects the whole document at parse time rather than
        // silently degrading.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        // No `kpar_digest`.
        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            format!(
                r#"{{"versions":[{{"version":"0.3.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":42}}]}}"#,
            ),
        );

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("missing required field must reject the document");
        assert!(
            matches!(
                err,
                super::IndexEnvironmentError::Fetch(super::HttpFetchError::JsonParse { .. })
            ),
            "expected Fetch(JsonParse), got {err:?}"
        );

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn duplicate_versions_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
        // Rejected at ingest rather than silently preserved — "pick the
        // better duplicate" has no principled answer when the two
        // entries might carry different digests.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("1.0.0", "[]"), ("1.0.0", "[]")]),
        );

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("duplicate versions must reject the document");
        match err {
            super::IndexEnvironmentError::DuplicateVersion { version, .. } => {
                assert_eq!(version, "1.0.0");
            }
            other => panic!("expected DuplicateVersion, got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_with_artifact_metadata_parses() -> Result<(), Box<dyn std::error::Error>> {
        // Unknown fields at both entry and document level must be
        // ignored — the forward-compat rule is load-bearing.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            r#"{
                    "versions": [
                        {
                            "version": "0.3.0",
                            "project_digest": "sha256:00000000000000000000000000000000000000000000000000000000deadbeef",
                            "kpar_size": 4096,
                            "kpar_digest": "sha256:00000000000000000000000000000000000000000000000000000000cafef00d",
                            "usage": [],
                            "some_future_field": "ignored"
                        }
                    ],
                    "another_future_field": ["also", "ignored"]
                }"#,
        );

        let versions: Vec<_> = env
            .versions(purl("admin/proj0"))?
            .collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["0.3.0"]);
        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn malformed_versions_json_surfaces_parse_error() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = server
            .mock("GET", "/admin/proj0/versions.json")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body>not json</body></html>")
            .create();

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("malformed JSON must error");
        match err {
            super::IndexEnvironmentError::Fetch(super::HttpFetchError::JsonParse {
                url, ..
            }) => {
                assert!(
                    url.contains("/admin/proj0/versions.json"),
                    "url carried: {url}"
                );
            }
            other => panic!("expected Fetch(JsonParse), got {other:?}"),
        }
        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_follows_redirect() -> Result<(), Box<dyn std::error::Error>> {
        // Pin the redirect-following invariant so a future
        // client-construction refactor that disables redirects
        // (`Policy::none()`) fails loudly.
        let mut server = mockito::Server::new();
        let env = test_env_sync(&server)?;

        let redirect_mock = server
            .mock("GET", "/admin/proj0/versions.json")
            .with_status(301)
            .with_header("location", "/redirected/versions.json")
            .expect(1)
            .create();

        let target_mock = mock_json_get(
            &mut server,
            "/redirected/versions.json",
            versions_json_body([("1.0.0", "[]")]),
        );

        let versions: Vec<_> = env
            .versions(purl("admin/proj0"))?
            .collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["1.0.0"]);

        redirect_mock.assert();
        target_mock.assert();

        Ok(())
    }
}

/// Tests for `get_project` — selecting a concrete version and exposing
/// its artifacts.
///
/// Rules exercised here (see module-level doc for cross-cutting ones):
/// - A `versions.json` 404 surfaces as `ProjectNotInIndex` (§8): the
///   project is not in this index, so a direct caller can tell "not
///   here" apart from a transport failure.
/// - A 404 on a per-version file (`.project.json`, `.meta.json`)
///   for an `available` / `yanked` entry remains a hard
///   `BadHttpStatus(404)`: §9 requires those files to exist whenever
///   the version is so listed.
/// - `.project.json` / `.meta.json` are the source of truth for
///   info/meta (not IRI heuristics and not the advertised usage).
/// - A caller-supplied version not listed in `versions.json` surfaces
///   as `VersionNotInIndex`; there is no kpar-only fallback.
mod get_project {
    use super::*;

    #[test]
    fn get_project_on_versions_json_404_surfaces_project_not_in_index()
    -> Result<(), Box<dyn std::error::Error>> {
        // §8 — a `versions.json` 404 means the project is not in this
        // index. `get_project_async` MUST surface a distinct
        // `ProjectNotInIndex` error so a direct caller can tell "not
        // here" apart from a transport failure or from the
        // version-listed-but-missing case
        // (`VersionNotInIndex`).
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = server
            .mock("GET", "/nope/nope/versions.json")
            .with_status(404)
            .with_body("not found")
            .create();

        let err = env
            .get_project(purl("nope/nope"), "1.0.0")
            .expect_err("versions.json 404 must surface as ProjectNotInIndex");
        match err {
            super::IndexEnvironmentError::ProjectNotInIndex { url, iri } => {
                assert_eq!(iri, purl("nope/nope"));
                assert!(url.contains("/versions.json"), "url carried: {url}");
            }
            other => panic!("expected ProjectNotInIndex, got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_on_versions_json_5xx_is_hard_error() -> Result<(), Box<dyn std::error::Error>> {
        // Direct `get_project` has the same §8 boundary as
        // `versions_async`: 404 is "not in this index", but 5xx is a
        // hard server failure.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = server
            .mock("GET", "/admin/proj0/versions.json")
            .with_status(503)
            .with_body("service unavailable")
            .create();

        let err = env
            .get_project(purl("admin/proj0"), "1.0.0")
            .expect_err("5xx on root versions.json must be a hard error");
        match err {
            super::IndexEnvironmentError::Fetch(super::HttpFetchError::BadHttpStatus {
                status,
                ..
            }) => {
                assert_eq!(status, reqwest::StatusCode::SERVICE_UNAVAILABLE);
            }
            other => panic!("expected BadHttpStatus(503), got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_on_removed_version_fails_with_version_removed()
    -> Result<(), Box<dyn std::error::Error>> {
        // §9 + §13 — when a lockfile replay (or any direct
        // `get_project`) hits a version whose `versions.json` entry
        // carries `status: "removed"`, the client MUST surface a
        // distinct "removed upstream" diagnostic rather than the
        // generic 404 on the per-version files. We assert the
        // `VersionRemoved` variant is produced before any file fetch
        // is issued — no `.project.json` / `.meta.json` / kpar mocks
        // are registered, so any unexpected fetch would surface as a
        // mockito miss.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let body = format!(
            r#"{{"versions":[
                {{"version":"0.3.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}","status":"removed"}}
            ]}}"#
        );

        let versions_mock = mock_json_get(&mut server, "/admin/proj0/versions.json", body);

        let err = env
            .get_project(purl("admin/proj0"), "0.3.0")
            .expect_err("removed entries must hard-fail with VersionRemoved");
        match err {
            super::IndexEnvironmentError::VersionRemoved { iri, version, url } => {
                assert_eq!(iri, purl("admin/proj0"));
                assert_eq!(version, "0.3.0");
                assert!(url.contains("/versions.json"), "url carried: {url}");
            }
            other => panic!("expected VersionRemoved, got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_on_yanked_version_still_serves_files() -> Result<(), Box<dyn std::error::Error>>
    {
        // §11 Retirement + §13 lockfile contract — a `yanked` entry's
        // per-version files remain served so existing lockfiles still
        // `sync` cleanly; only new resolutions are affected (and those
        // are filtered at the `versions_async` boundary in §12). Direct
        // `get_project` on a yanked version MUST behave exactly like
        // `available`.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let info_json = project_json_body("proj0", Some("admin"), "0.3.0", "[]");
        let meta_json = meta_json_body();
        let project_digest = project_digest(&info_json, meta_json)?;

        let body = format!(
            r#"{{"versions":[
                {{"version":"0.3.0","usage":[],"project_digest":"{project_digest}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}","status":"yanked"}}
            ]}}"#
        );

        let versions_mock = mock_json_get(&mut server, "/admin/proj0/versions.json", body);
        let project_json_mock =
            mock_json_get(&mut server, "/admin/proj0/0.3.0/.project.json", info_json);
        let meta_json_mock = mock_json_get(&mut server, "/admin/proj0/0.3.0/.meta.json", meta_json);

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let (info, _meta) = project.get_project()?;
        let info = info.expect("info should be prefetched for yanked entries too");
        assert_eq!(info.name, "proj0");
        assert_eq!(info.version, "0.3.0");

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_sysand_purl_route() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let info_json = project_json_body("proj0", Some("admin"), "0.3.0", "[]");
        let meta_json = meta_json_body();
        let project_digest = project_digest(&info_json, meta_json)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body_with_project_digest([("0.3.0", "[]", &project_digest)]),
        );

        let project_json_mock =
            mock_json_get(&mut server, "/admin/proj0/0.3.0/.project.json", info_json);

        let meta_json_mock = mock_json_get(&mut server, "/admin/proj0/0.3.0/.meta.json", meta_json);

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;

        let inner = &project.inner;
        assert_eq!(
            inner.archive.url.as_str(),
            format!("{}/admin/proj0/0.3.0/project.kpar", server.url())
        );

        // get_project should return the prefetched info without hitting the
        // kpar URL (no mock for the kpar body).
        let (info, meta) = project.get_project()?;
        let info = info.expect("info should be prefetched");
        assert_eq!(info.name, "proj0");
        assert_eq!(info.publisher.as_deref(), Some("admin"));
        assert_eq!(info.version, "0.3.0");
        assert!(info.usage.is_empty());
        assert!(meta.is_some());

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_iri_hash_route() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let info_json = project_json_body("b", None, "1.0.0", "[]");
        let meta_json = meta_json_body();
        let project_digest = project_digest(&info_json, meta_json)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/_iri/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/versions.json",
            versions_json_body_with_project_digest([("1.0.0", "[]", &project_digest)]),
        );

        let project_json_mock = mock_json_get(
            &mut server,
            "/_iri/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/1.0.0/.project.json",
            info_json,
        );

        let meta_json_mock = mock_json_get(
            &mut server,
            "/_iri/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/1.0.0/.meta.json",
            meta_json,
        );

        let project = env.get_project("urn:kpar:b", "1.0.0")?;

        let inner = &project.inner;
        assert_eq!(
            inner.archive.url.as_str(),
            format!(
                "{}/_iri/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/1.0.0/project.kpar",
                server.url()
            )
        );

        let (info, _) = project.get_project()?;
        let info = info.expect("info should be prefetched");
        assert_eq!(info.name, "b");
        assert_eq!(info.publisher, None);

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_carries_usage() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let usage_json = format!(
            r#"[
            {{"resource":"{PKG_SYSAND_PREFIX}admin/dep","versionConstraint":"<2"}},
            {{"resource":"{PKG_SYSAND_PREFIX}admin/other"}}
        ]"#
        );
        let info_json = project_json_body("proj0", Some("admin"), "0.3.0", &usage_json);
        let meta_json = meta_json_body();
        let project_digest = project_digest(&info_json, meta_json)?;

        // `usage` shown to the caller comes from `.project.json`; carry
        // the same payload in `versions.json` to check there's no
        // double-merge or cross-source mixing.
        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body_with_project_digest([("0.3.0", &usage_json, &project_digest)]),
        );

        let project_json_mock =
            mock_json_get(&mut server, "/admin/proj0/0.3.0/.project.json", info_json);

        let meta_json_mock = mock_json_get(&mut server, "/admin/proj0/0.3.0/.meta.json", meta_json);

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let (info, _) = project.get_project()?;
        let info = info.expect("info should be prefetched");

        assert_eq!(info.usage.len(), 2);
        assert_eq!(info.usage[0].resource, purl("admin/dep"));
        assert_eq!(info.usage[0].version_constraint.as_deref(), Some("<2"));
        assert_eq!(info.usage[1].resource, purl("admin/other"));
        assert_eq!(info.usage[1].version_constraint, None);

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_ignores_textual_usage_drift() -> Result<(), Box<dyn std::error::Error>> {
        // Regression guard: textual drift between advertised and fetched
        // `usage` must be ignored — the server is authoritative, and a
        // drifted textual field would produce a different canonical
        // digest anyway, which is what verification actually compares.
        // This test pins that behavior against a regression toward
        // hard-failing on textual drift.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let advertised_usage = format!(
            r#"[
            {{"resource":"{PKG_SYSAND_PREFIX}admin/dep","versionConstraint":"<2"}}
        ]"#
        );
        let fetched_usage = format!(
            r#"[
            {{"resource":"{PKG_SYSAND_PREFIX}admin/dep","versionConstraint":"<3"}}
        ]"#
        );
        // Compute the digest against the fetched (info, meta) so
        // verification passes; advertised and fetched `usage` differ.
        let fetched_info_json = project_json_body("proj0", Some("admin"), "0.3.0", &fetched_usage);
        let meta_json = meta_json_body();
        let advertised_digest = project_digest(&fetched_info_json, meta_json)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body_with_project_digest([(
                "0.3.0",
                &advertised_usage,
                &advertised_digest,
            )]),
        );

        let project_json_mock = mock_json_get(
            &mut server,
            "/admin/proj0/0.3.0/.project.json",
            fetched_info_json,
        );

        let meta_json_mock = mock_json_get(&mut server, "/admin/proj0/0.3.0/.meta.json", meta_json);

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let info = project
            .get_info()
            .expect("textual usage drift must be ignored; server is authoritative")
            .expect("info must be present");
        assert_eq!(info.name, "proj0");

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_returns_real_info_meta_from_per_version_files()
    -> Result<(), Box<dyn std::error::Error>> {
        // Fixture uses values no IRI-derived heuristic could invent,
        // so a regression that re-introduces synthesis would fail.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let usage_json =
            format!(r#"[{{"resource":"{PKG_SYSAND_PREFIX}x/y","versionConstraint":">=1"}}]"#);
        let info_json = project_json_body(
            "real_name_from_server",
            Some("real_publisher"),
            "0.3.0",
            &usage_json,
        );
        let meta_json = r#"{"index":{},"created":"2026-04-17T00:00:00.000000000Z","metamodel":"https://www.omg.org/spec/KerML/20250201"}"#;
        let project_digest = project_digest(&info_json, meta_json)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body_with_project_digest([("0.3.0", &usage_json, &project_digest)]),
        );

        let project_json_mock =
            mock_json_get(&mut server, "/admin/proj0/0.3.0/.project.json", info_json);

        let meta_json_mock = mock_json_get(&mut server, "/admin/proj0/0.3.0/.meta.json", meta_json);

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let (info, meta) = project.get_project()?;
        let info = info.expect("info should be prefetched");
        let meta = meta.expect("meta should be prefetched");

        assert_eq!(info.name, "real_name_from_server");
        assert_eq!(info.publisher.as_deref(), Some("real_publisher"));
        assert_eq!(info.version, "0.3.0");
        assert_eq!(info.usage.len(), 1);
        assert_eq!(info.usage[0].resource, purl("x/y"));
        assert_eq!(info.usage[0].version_constraint.as_deref(), Some(">=1"));

        assert_eq!(
            meta.metamodel.as_deref(),
            Some("https://www.omg.org/spec/KerML/20250201")
        );

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_missing_per_version_info_errors() -> Result<(), Box<dyn std::error::Error>> {
        // `.project.json` missing while the version is listed: must
        // surface as a hard error, not silently proceed without info.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("0.3.0", "[]")]),
        );

        let project_json_mock = server
            .mock("GET", "/admin/proj0/0.3.0/.project.json")
            .with_status(404)
            .expect(1)
            .create();

        let _meta_json_mock = server
            .mock("GET", "/admin/proj0/0.3.0/.meta.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(meta_json_body())
            .create();

        // `env.get_project` returns a lazy wrapper; forcing `get_project()` on it
        // triggers the per-version `.project.json` fetch that must surface the
        // 404 as a hard error.
        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let err = project
            .get_project()
            .expect_err("missing .project.json must surface as a hard error");
        let text = format!("{err:?}");
        assert!(
            text.contains("BadHttpStatus") && text.contains("/admin/proj0/0.3.0/.project.json"),
            "expected BadHttpStatus on .project.json, got: {text}"
        );

        versions_mock.assert();
        project_json_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_missing_per_version_meta_errors() -> Result<(), Box<dyn std::error::Error>> {
        // Parallel to the `.project.json`-missing case, on the other
        // leg: `.meta.json` 404 with `.project.json` serving cleanly
        // must not silently expose partial state.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("0.3.0", "[]")]),
        );

        let _project_json_mock = server
            .mock("GET", "/admin/proj0/0.3.0/.project.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(project_json_body("proj0", Some("admin"), "0.3.0", "[]"))
            .create();

        let meta_json_mock = server
            .mock("GET", "/admin/proj0/0.3.0/.meta.json")
            .with_status(404)
            .expect(1)
            .create();

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let err = project
            .get_project()
            .expect_err("missing .meta.json must surface as a hard error");
        let text = format!("{err:?}");
        assert!(
            text.contains("BadHttpStatus") && text.contains("/admin/proj0/0.3.0/.meta.json"),
            "expected BadHttpStatus on .meta.json, got: {text}"
        );

        versions_mock.assert();
        meta_json_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_version_not_in_versions_json_errors() -> Result<(), Box<dyn std::error::Error>> {
        // A version not in `versions.json` must surface as
        // `VersionNotInIndex` — no kpar-only fallback.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("0.3.0", "[]")]),
        );

        let err = env
            .get_project(purl("admin/proj0"), "9.9.9")
            .expect_err("requesting an absent version must error");
        match err {
            super::IndexEnvironmentError::VersionNotInIndex { url, version } => {
                assert!(
                    url.contains("/admin/proj0/versions.json"),
                    "url carried: {url}"
                );
                assert_eq!(version, "9.9.9");
            }
            other => panic!("expected VersionNotInIndex, got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }
}

/// Tests that the `_iri/<sha256_hex(normalized_iri)>/` bucket is
/// computed from the RFC-3986-normalized IRI — so two byte-distinct
/// but semantically equivalent IRIs land in the same bucket. Each
/// test seeds the mock for the normalized form and issues a
/// non-normalized request; a missing normalization step would miss
/// the mock and fail `expect(1)`.
mod iri {
    use super::*;

    #[test]
    fn iri_hash_bucket_applies_rfc3986_normalization() -> Result<(), Box<dyn std::error::Error>> {
        // Scheme/host case and percent-encoding case: `HTTP://Example.COM`
        // and lowercase `%7e` must converge on the normalized form.
        // `%7E` decodes to `~` (unreserved per RFC 3986 §2.3), so the
        // normalized form carries the literal tilde rather than
        // preserving the percent-escape.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        // Compute the expected bucket hash rather than hard-coding it,
        // so the test documents its own derivation.
        let normalized_iri = "http://example.com/~user";
        let raw_request_iri = "HTTP://Example.COM/%7euser";

        use crate::env::iri_normalize::normalize_iri_for_hash;
        let parsed = fluent_uri::Iri::parse(raw_request_iri)?;
        assert_eq!(normalize_iri_for_hash(parsed)?.as_str(), normalized_iri);

        // Compute what the env will look up.
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(normalized_iri);
        let expected_hash = format!("{:x}", h.finalize());

        let versions_mock = mock_json_get(
            &mut server,
            &format!("/_iri/{expected_hash}/versions.json"),
            versions_json_body([("1.0.0", "[]")]),
        );

        // Ask with the raw form; normalization must collapse it onto the
        // normalized bucket.
        let versions: Vec<_> = env.versions(raw_request_iri)?.collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["1.0.0"]);

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn iri_hash_bucket_strips_default_port() -> Result<(), Box<dyn std::error::Error>> {
        // Scheme-default port stripping: `http://example.com:80/` and
        // `http://example.com/` must hash to the same bucket.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        use sha2::{Digest, Sha256};
        let canonical = "http://example.com/";
        let mut h = Sha256::new();
        h.update(canonical);
        let expected_hash = format!("{:x}", h.finalize());

        let versions_mock = mock_json_get(
            &mut server,
            &format!("/_iri/{expected_hash}/versions.json"),
            versions_json_body([("1.0.0", "[]")]),
        );

        let versions: Vec<_> = env
            .versions("http://example.com:80/")?
            .collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["1.0.0"]);

        versions_mock.assert();

        Ok(())
    }
}

/// Tests for digest verification — both the pre-expose check on
/// `.project.json` / `.meta.json` and the post-download authoritative
/// check on the kpar-derived canonical digest.
///
/// Invariants these tests pin (see module-level doc for the overall
/// digest-verification rule):
/// - The canonical project digest is computable from `.project.json`
///   and `.meta.json` alone; a non-SHA256 `meta.checksum` entry
///   breaks that property and the client refuses to expose either
///   document.
/// - Canonicalization lowercases SHA256 hex; a mixed-case meta entry
///   must not produce a spurious drift.
/// - When the advertised digest is usable, `checksum_canonical_hex`
///   returns it without fetching any leaf artifact (including the
///   kpar). Per-version endpoints in these tests use `expect(0)`
///   so a regression that re-introduces unnecessary fetches fails
///   loudly.
/// - Digest shape (`sha256:<64-hex>`) is validated at ingest on
///   `versions.json` itself, not just when a specific version is
///   materialized — the cache is shared, so deferring validation
///   would let `versions_async` hand out strings from a document
///   that `get_project_async` will later reject.
mod digest {
    use super::*;

    #[test]
    fn advertised_project_digest_mismatch_rejected_before_expose()
    -> Result<(), Box<dyn std::error::Error>> {
        // Syntactically-valid-but-wrong advertised digest: must refuse
        // to expose info AND meta, even though the JSON pair itself
        // parses cleanly.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let info_json = project_json_body("proj0", Some("admin"), "0.3.0", "[]");
        let meta_json = meta_json_body();
        let bogus_advertised_digest = format!("sha256:{}", "c".repeat(64));

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body_with_project_digest([("0.3.0", "[]", &bogus_advertised_digest)]),
        );

        let project_json_mock = mock_json_get_count(
            &mut server,
            "/admin/proj0/0.3.0/.project.json",
            info_json,
            2,
        );

        let meta_json_mock =
            mock_json_get_count(&mut server, "/admin/proj0/0.3.0/.meta.json", meta_json, 2);

        // Verification runs from JSON only.
        let kpar_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/project.kpar");

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let err = project
            .get_info()
            .expect_err("digest drift must reject before exposing info");
        match err {
            IndexEntryProjectError::AdvertisedDigestDrift { expected, .. } => {
                assert_eq!(expected, "c".repeat(64));
            }
            other => panic!("expected AdvertisedDigestDrift, got {other:?}"),
        }

        // get_meta() must also refuse — both documents must be unavailable.
        let err_meta = project
            .get_meta()
            .expect_err("digest drift must reject before exposing meta too");
        assert!(
            matches!(
                err_meta,
                IndexEntryProjectError::AdvertisedDigestDrift { .. }
            ),
            "get_meta must also surface drift"
        );

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();
        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn checksum_uses_inline_project_digest_and_skips_kpar_download()
    -> Result<(), Box<dyn std::error::Error>> {
        // The pre-download shortcut: `checksum_canonical_hex` returns
        // the advertised digest without touching any leaf endpoint.
        // A regression that re-introduced a materialization step here
        // would silently start downloading archives during resolution.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let expected_hex = "b".repeat(64);
        let advertised_digest = format!("sha256:{expected_hex}");

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            format!(
                r#"{{"versions":[{{"version":"0.3.0","usage":[],"project_digest":"{advertised_digest}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}"}}]}}"#,
            ),
        );

        let project_json_mock =
            expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.project.json");

        let meta_json_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.meta.json");

        let kpar_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/project.kpar");

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let digest = project
            .checksum_canonical_hex()?
            .expect("prefetched digest should propagate");
        assert_eq!(digest, expected_hex);

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();
        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn malformed_project_digest_errors() -> Result<(), Box<dyn std::error::Error>> {
        // Non-`sha256:<hex>` advertised value: surface as a protocol
        // error rather than silently recomputing (which would break
        // lock/sync cross-checks downstream).
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            format!(
                r#"{{"versions":[{{"version":"0.3.0","usage":[],"project_digest":"md5:abc","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}"}}]}}"#,
            ),
        );

        let kpar_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/project.kpar");

        let err = env
            .get_project(purl("admin/proj0"), "0.3.0")
            .expect_err("malformed project_digest must surface as a protocol error");
        match err {
            super::IndexEnvironmentError::InvalidVersionEntry {
                version,
                field,
                value,
                ..
            } => {
                assert_eq!(version, "0.3.0");
                assert_eq!(field, "project_digest");
                assert_eq!(value, "md5:abc");
            }
            other => panic!("expected InvalidVersionEntry, got {other:?}"),
        }

        versions_mock.assert();
        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn project_digest_drift_after_download_errors() -> Result<(), Box<dyn std::error::Error>> {
        // Post-download authoritative check: correct `kpar_digest` but
        // deliberately-wrong `project_digest` lets the download succeed
        // and forces `checksum_canonical_hex` into the reconciliation
        // branch, where the mismatch must surface as
        // `AdvertisedDigestDrift` rather than silently corrupt the
        // lockfile.
        use sha2::{Digest as _, Sha256};

        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        // `.project.json` / `.meta.json` are only consumed from inside the
        // kpar for this test; the destructured JSON strings are unused.
        let (kpar_bytes, _info_json, _meta_json) =
            build_minimal_kpar("proj0", "0.3.0", "foo.sysml", "// hi");
        let kpar_digest_hex = format!("{:x}", Sha256::digest(&kpar_bytes));
        let advertised_kpar = format!("sha256:{kpar_digest_hex}");

        // `bbb…b` is not the canonical project digest of the archive above,
        // which is what forces the drift branch post-download.
        let wrong_project_digest_hex = "b".repeat(64);
        let advertised_project = format!("sha256:{wrong_project_digest_hex}");

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            format!(
                r#"{{"versions":[{{"version":"0.3.0","usage":[],"project_digest":"{advertised_project}","kpar_size":{kpar_size},"kpar_digest":"{advertised_kpar}"}}]}}"#,
                kpar_size = kpar_bytes.len(),
            ),
        );

        // Reconciliation runs against the in-archive copies; neither
        // `read_source` nor the post-download branch of
        // `checksum_canonical_hex` touches the per-version JSON
        // endpoints. `expect(0)` catches a regression that would fall
        // back to those during drift checks.
        let project_json_mock =
            expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.project.json");

        let meta_json_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.meta.json");

        let kpar_mock = server
            .mock("GET", "/admin/proj0/0.3.0/project.kpar")
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body(&kpar_bytes)
            .expect(1)
            .create();

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;

        // Force a download so `checksum_canonical_hex` reaches the
        // post-download (authoritative local) branch rather than the
        // pre-download shortcut.
        let mut reader = project.read_source("foo.sysml").unwrap();
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut reader, &mut buf)?;
        drop(reader);

        let err = project
            .checksum_canonical_hex()
            .expect_err("drift between advertised and locally-computed digest must error");

        // Surface path: CanonicalizationError::ProjectRead ->
        // IndexEntryProjectError::AdvertisedDigestDrift. Traverse whatever
        // wrappers the display-side adds by matching on the debug text.
        let text = format!("{err:?}");
        assert!(
            text.contains("AdvertisedDigestDrift"),
            "expected AdvertisedDigestDrift, got: {text}"
        );
        assert!(
            text.contains(&wrong_project_digest_hex),
            "advertised digest should appear in error: {text}"
        );

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();
        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_async_rejects_document_with_malformed_digest()
    -> Result<(), Box<dyn std::error::Error>> {
        // Shape-check happens at `versions.json` parse time. Because
        // `versions_async` and `get_project_async` share the cache, a
        // deferred check would let `versions_async` happily stream
        // strings from a document that a later `get_project_async`
        // would reject — a surprise much later.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            format!(
                r#"{{"versions":[{{"version":"0.3.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":42,"kpar_digest":"md5:abc"}}]}}"#,
            ),
        );

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("malformed kpar_digest must reject the document at parse time");
        match err {
            super::IndexEnvironmentError::InvalidVersionEntry {
                version,
                field,
                value,
                ..
            } => {
                assert_eq!(version, "0.3.0");
                assert_eq!(field, "kpar_digest");
                assert_eq!(value, "md5:abc");
            }
            other => panic!("expected InvalidVersionEntry, got {other:?}"),
        }

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_accepts_canonical_digest_with_mixed_case_sha256_meta()
    -> Result<(), Box<dyn std::error::Error>> {
        // Regression guard: the old raw-hash reconciliation hashed
        // `.meta.json` as-written and produced a spurious
        // `AdvertisedDigestDrift` against the server's canonical
        // (lowercased) digest. The fix uses the canonical-inline
        // digest on both sides; this happy path must succeed without
        // downloading the kpar.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let info_json = project_json_body("proj0", Some("admin"), "0.3.0", "[]");
        // 64-char hex with uppercase digits — a legal SHA256 value that
        // canonicalization lowercases before hashing.
        let meta_json = r#"{"index":{"Sym":"foo.sysml"},"created":"2026-01-01T00:00:00.000000000Z","checksum":{"foo.sysml":{"value":"ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789","algorithm":"SHA256"}}}"#;
        let advertised_digest = canonical_project_digest(&info_json, meta_json)?;
        // Sanity: canonical and raw-hash digests must differ, else the
        // test would pass even with the old buggy code.
        assert_ne!(
            advertised_digest,
            project_digest(&info_json, meta_json)?,
            "fixture must exercise canonicalization — raw and canonical digests should differ"
        );

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body_with_project_digest([("0.3.0", "[]", &advertised_digest)]),
        );

        let project_json_mock =
            mock_json_get(&mut server, "/admin/proj0/0.3.0/.project.json", info_json);

        let meta_json_mock = mock_json_get(&mut server, "/admin/proj0/0.3.0/.meta.json", meta_json);

        let kpar_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/project.kpar");

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let (info, meta) = project
            .get_project()
            .expect("canonical digest reconciliation must succeed for mixed-case SHA256 meta");
        assert_eq!(info.as_ref().map(|i| i.name.as_str()), Some("proj0"));
        assert!(meta.is_some());

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();
        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn get_project_rejects_non_sha256_meta_checksums() -> Result<(), Box<dyn std::error::Error>> {
        // A non-SHA256 `meta.checksum` entry makes the canonical digest
        // require source reads. The index protocol requires verification
        // from (info, meta) alone, so the client refuses to expose either
        // document rather than silently skipping verification.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let info_json = project_json_body("proj0", Some("admin"), "0.3.0", "[]");
        // SHA1 — canonicalizing this entry would force reading source
        // bytes from the kpar, which the protocol forbids.
        let meta_json = r#"{"index":{"Sym":"foo.sysml"},"created":"2026-01-01T00:00:00.000000000Z","checksum":{"foo.sysml":{"value":"dabe95d26be5d1c68a80fae65d12ae056e8fc8ab","algorithm":"SHA1"}}}"#;

        let advertised_digest = format!("sha256:{}", "a".repeat(64));

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body_with_project_digest([("0.3.0", "[]", &advertised_digest)]),
        );

        let project_json_mock =
            mock_json_get(&mut server, "/admin/proj0/0.3.0/.project.json", info_json);

        let meta_json_mock = mock_json_get(&mut server, "/admin/proj0/0.3.0/.meta.json", meta_json);

        // Error is triggered purely on the JSON pair.
        let kpar_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/project.kpar");

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let err = project
            .get_info()
            .expect_err("non-SHA256 meta.checksum must refuse to expose info/meta");
        match err {
            IndexEntryProjectError::ProjectDigestRequiresSourceReads { .. } => {}
            other => panic!("expected ProjectDigestRequiresSourceReads, got {other:?}"),
        }

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();
        kpar_mock.assert();

        Ok(())
    }
}

/// Tests for the two caching layers: `fetched_info_meta`'s per-project
/// `OnceCell` (info/meta fetch+verify fan-in) and the env-scoped
/// `versions_cache` (per-IRI `versions.json` fan-in). Failures — including
/// 404s — are not cached, so retries re-issue the fetch.
mod caching {
    use super::*;

    #[test]
    fn info_and_meta_each_fetched_at_most_once_across_accessors()
    -> Result<(), Box<dyn std::error::Error>> {
        // `expect(1)` pins the single-fetch-per-document fan-in: a
        // regression that dropped the cache (separate OnceCells per
        // accessor, or fetch-on-use) would trip it.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let info_json = project_json_body("proj0", Some("admin"), "0.3.0", "[]");
        let meta_json = meta_json_body();
        let advertised_digest = project_digest(&info_json, meta_json)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body_with_project_digest([("0.3.0", "[]", &advertised_digest)]),
        );

        let project_json_mock = server
            .mock("GET", "/admin/proj0/0.3.0/.project.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(info_json)
            .expect(1)
            .create();

        let meta_json_mock = server
            .mock("GET", "/admin/proj0/0.3.0/.meta.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(meta_json)
            .expect(1)
            .create();

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        // Hit each accessor multiple times in mixed order; cache must hold.
        let _ = project.get_info()?;
        let _ = project.get_meta()?;
        let _ = project.get_project()?;
        let _ = project.get_info()?;
        let _ = project.get_meta()?;

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_fetched_once_per_env_lifetime() -> Result<(), Box<dyn std::error::Error>> {
        // Both `versions_async` and `get_project_async` read
        // `versions.json`; cached at the env level the document is
        // fetched once regardless of how many candidates the solver
        // touches.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = server
            .mock("GET", "/admin/proj0/versions.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(versions_json_body([("0.3.0", "[]")]))
            .expect(1)
            .create();

        // get_project also reaches `.project.json` / `.meta.json`; mock them
        // so the call succeeds, but they're orthogonal to the versions.json
        // cache assertion (no expectation on count).
        let _project_json_mock = server
            .mock("GET", "/admin/proj0/0.3.0/.project.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(project_json_body("proj0", Some("admin"), "0.3.0", "[]"))
            .create();
        let _meta_json_mock = server
            .mock("GET", "/admin/proj0/0.3.0/.meta.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(meta_json_body())
            .create();

        // Three independent calls into paths that consult versions.json.
        let _ = env.versions(purl("admin/proj0"))?.collect::<Vec<_>>();
        let _ = env.versions(purl("admin/proj0"))?.collect::<Vec<_>>();
        let _ = env.get_project(purl("admin/proj0"), "0.3.0")?;

        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn versions_json_404_refetched_on_every_call() -> Result<(), Box<dyn std::error::Error>> {
        // The "not in this index" 404 outcome (§8) is intentionally
        // not cached — the cache slot is only populated on a
        // successful validate — so a later publish to the same index
        // becomes visible without restart. Each retry re-issues the
        // fetch even though the resolver boundary surfaces it as an
        // empty stream.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = server
            .mock("GET", "/nope/nope/versions.json")
            .with_status(404)
            .expect(2)
            .create();

        let vs1: Vec<_> = env.versions(purl("nope/nope"))?.collect();
        let vs2: Vec<_> = env.versions(purl("nope/nope"))?.collect();
        assert!(
            vs1.into_iter().filter_map(Result::ok).next().is_none(),
            "first call must yield empty stream"
        );
        assert!(
            vs2.into_iter().filter_map(Result::ok).next().is_none(),
            "retry must yield empty stream (404 not cached)"
        );

        versions_mock.assert();

        Ok(())
    }
}

/// Tests for `read_source` / `sources` — the kpar-backed paths.
///
/// Shared shape these tests pin:
/// - Mismatch between advertised `kpar_digest` and served bytes
///   surfaces as `DigestMismatch` and never leaves a usable archive
///   on disk (so retries re-download and re-verify).
/// - `sources_async` takes `kpar_size` from the inline advertised
///   value and MUST NOT issue a HEAD.
/// - `read_source` and `sources` are purely archive-backed — the
///   per-version `.project.json` / `.meta.json` endpoints are not
///   on this code path, and every test here pins that with
///   `expect(0)` so a regression that reaches for them fails
///   loudly.
mod sources {
    use super::*;

    #[test]
    fn kpar_digest_mismatch_surfaces_error() -> Result<(), Box<dyn std::error::Error>> {
        // First call that forces `ensure_downloaded` (here
        // `read_source`) must surface `DigestMismatch` rather than
        // silently accepting the archive.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        // An advertised digest that doesn't match the body bytes below.
        let advertised_digest_hex = "a".repeat(64);
        let advertised = format!("sha256:{advertised_digest_hex}");
        let kpar_body: &[u8] = b"not really a kpar";

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            format!(
                r#"{{"versions":[{{"version":"0.3.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":{},"kpar_digest":"{advertised}"}}]}}"#,
                kpar_body.len()
            ),
        );

        let project_json_mock =
            expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.project.json");

        let meta_json_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.meta.json");

        let kpar_mock = server
            .mock("GET", "/admin/proj0/0.3.0/project.kpar")
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body(kpar_body)
            .expect(1)
            .create();

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let err = project
            .read_source("anything.sysml")
            .err()
            .expect("mismatched kpar digest must error");
        match err {
            IndexEntryProjectError::Downloaded(ReqwestKparDownloadedError::DigestMismatch {
                expected,
                ..
            }) => {
                assert_eq!(expected, advertised_digest_hex);
            }
            other => panic!("expected DigestMismatch, got {other:?}"),
        }

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();
        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn kpar_digest_mismatch_does_not_persist_archive() -> Result<(), Box<dyn std::error::Error>> {
        // Defense in depth: the atomic-rename path must never install
        // a mismatched body at `archive_path`, so a retry re-downloads
        // rather than short-circuiting on a stale tampered file.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let advertised_digest_hex = "0".repeat(64);
        let advertised = format!("sha256:{advertised_digest_hex}");
        let kpar_body: &[u8] = b"not really a kpar";

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            format!(
                r#"{{"versions":[{{"version":"0.3.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":{},"kpar_digest":"{advertised}"}}]}}"#,
                kpar_body.len(),
            ),
        );

        let project_json_mock =
            expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.project.json");

        let meta_json_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.meta.json");

        let kpar_mock = server
            .mock("GET", "/admin/proj0/0.3.0/project.kpar")
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body(kpar_body)
            .expect(1)
            .create();

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let _ = project
            .read_source("anything.sysml")
            .err()
            .expect("mismatched kpar digest must error");

        assert!(
            !project.inner.archive.is_downloaded_and_verified(),
            "tampered archive must not be reported as verified",
        );

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();
        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn kpar_digest_match_allows_download() -> Result<(), Box<dyn std::error::Error>> {
        // Matching `kpar_digest`: verification passes and the eventual
        // error is a downstream kpar-parser error on the intentionally
        // invalid zip body — the absence of `DigestMismatch` is what
        // proves the digest check passed.
        use sha2::{Digest as _, Sha256};

        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let body: &[u8] = b"not really a kpar either";
        let actual_hex = format!("{:x}", Sha256::digest(body));
        let advertised = format!("sha256:{actual_hex}");

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            format!(
                r#"{{"versions":[{{"version":"0.3.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":42,"kpar_digest":"{advertised}"}}]}}"#,
            ),
        );

        let project_json_mock =
            expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.project.json");

        let meta_json_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.meta.json");

        let kpar_mock = server
            .mock("GET", "/admin/proj0/0.3.0/project.kpar")
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body(body)
            .expect(1)
            .create();

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let err = project
            .read_source("anything.sysml")
            .err()
            .expect("invalid zip must error, but not with DigestMismatch");
        if matches!(
            err,
            IndexEntryProjectError::Downloaded(ReqwestKparDownloadedError::DigestMismatch { .. })
        ) {
            panic!("digest matched; DigestMismatch should not surface");
        }

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();
        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn sources_async_uses_inline_kpar_size_and_skips_head() -> Result<(), Box<dyn std::error::Error>>
    {
        // Pin the no-HEAD invariant: `sources_async` must read
        // `kpar_size` from `versions.json`, not probe the archive.
        let mut server = mockito::Server::new();

        let env = test_env_sync(&server)?;

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            format!(
                r#"{{"versions":[{{"version":"0.3.0","usage":[],"project_digest":"{FILLER_DIGEST}","kpar_size":42,"kpar_digest":"{FILLER_DIGEST}"}}]}}"#,
            ),
        );

        let project_json_mock =
            expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.project.json");

        let meta_json_mock = expect_untouched(&mut server, "GET", "/admin/proj0/0.3.0/.meta.json");

        let head_mock = expect_untouched(&mut server, "HEAD", "/admin/proj0/0.3.0/project.kpar");

        let project = env.get_project(purl("admin/proj0"), "0.3.0")?;
        let sources = project.sources(&ProjectContext::default())?;

        assert_eq!(sources.len(), 1);
        match &sources[0] {
            Source::IndexKpar {
                index_kpar_size, ..
            } => assert_eq!(index_kpar_size.get(), 42),
            other => panic!("expected Source::IndexKpar, got {:?}", other),
        }

        versions_mock.assert();
        project_json_mock.assert();
        meta_json_mock.assert();
        head_mock.assert();

        Ok(())
    }
}

/// Tests for discovery of `index_root` / `api_root` via
/// `sysand-index-config.json`.
///
/// Discovery-specific rules (see module-level doc for cross-cutting
/// ones):
/// - 200 parses the document; 404 treats the discovery document as
///   absent and defaults both roots to the discovery root; any other
///   non-2xx is a hard error.
/// - `index_root` and `api_root` MUST be absolute URLs; relative
///   values are rejected rather than resolved against the discovery
///   root (avoids redirect-dependent resolution).
mod discovery {
    use super::*;

    #[test]
    fn discovery_absent_config_defaults_to_discovery_root() -> Result<(), Box<dyn std::error::Error>>
    {
        // Index reads proceed against the discovery root when the
        // discovery document is absent.
        let mut server = mockito::Server::new();

        let config_mock = server
            .mock("GET", "/sysand-index-config.json")
            .with_status(404)
            .expect(1)
            .create();

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("1.0.0", "[]")]),
        );

        let env = test_env_sync_discovery(&server)?;

        let versions: Vec<_> = env
            .versions(purl("admin/proj0"))?
            .collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["1.0.0"]);

        config_mock.assert();
        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn discovery_config_remaps_index_root() -> Result<(), Box<dyn std::error::Error>> {
        // An `index_root` in the discovery document redirects index
        // reads to the remapped base. The discovery root has no
        // matching path, so a regression that ignored the remap would
        // 404 here.
        let mut server = mockito::Server::new();

        let config_body = format!(r#"{{"index_root":"{}/index/"}}"#, server.url());
        let config_mock = mock_json_get(&mut server, "/sysand-index-config.json", config_body);

        let remap_mock = mock_json_get(
            &mut server,
            "/index/admin/proj0/versions.json",
            versions_json_body([("1.0.0", "[]")]),
        );

        let env = test_env_sync_discovery(&server)?;

        let versions: Vec<_> = env
            .versions(purl("admin/proj0"))?
            .collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["1.0.0"]);

        config_mock.assert();
        remap_mock.assert();

        Ok(())
    }

    #[test]
    fn discovery_rejects_relative_index_root() -> Result<(), Box<dyn std::error::Error>> {
        // Relative `index_root` -> `RelativeUrl` error. Discovery is
        // resolved at env construction, so the rejection surfaces from
        // `test_env_sync_discovery` rather than from a later
        // `versions()` call.
        let mut server = mockito::Server::new();

        let config_mock = mock_json_get(
            &mut server,
            "/sysand-index-config.json",
            r#"{"index_root":"/index/"}"#,
        );

        let err = test_env_sync_discovery(&server).expect_err("relative index_root must reject");
        let text = format!("{err:?}");
        assert!(
            text.contains("RelativeUrl") && text.contains("index_root"),
            "expected RelativeUrl on index_root, got: {text}"
        );

        config_mock.assert();

        Ok(())
    }

    #[test]
    fn discovery_rejects_userinfo_index_root() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let config_mock = mock_json_get(
            &mut server,
            "/sysand-index-config.json",
            r#"{"index_root":"https://user:password@example.com/index/"}"#,
        );

        let err = test_env_sync_discovery(&server).expect_err("userinfo index_root must reject");
        let text = format!("{err:?}");
        assert!(
            text.contains("Userinfo") && text.contains("index_root"),
            "expected Userinfo on index_root, got: {text}"
        );

        config_mock.assert();

        Ok(())
    }

    #[test]
    fn discovery_5xx_is_hard_error() -> Result<(), Box<dyn std::error::Error>> {
        // A broken server and a misconfigured base URL are
        // indistinguishable, so anything beyond 200/404 is a hard
        // error. With eager discovery this surfaces from env
        // construction.
        let mut server = mockito::Server::new();

        let config_mock = server
            .mock("GET", "/sysand-index-config.json")
            .with_status(503)
            .expect(1)
            .create();

        let err =
            test_env_sync_discovery(&server).expect_err("5xx on discovery must be a hard error");
        let text = format!("{err:?}");
        assert!(
            text.contains("BadHttpStatus") && text.contains("503"),
            "expected BadHttpStatus(503) on discovery, got: {text}"
        );

        config_mock.assert();

        Ok(())
    }

    #[test]
    fn discovery_remapped_versions_5xx_is_hard_error() -> Result<(), Box<dyn std::error::Error>> {
        // Companion to `test_discovery_5xx_is_hard_error` (which covers a
        // 5xx on the discovery endpoint itself). Here the discovery
        // document succeeds and remaps `index_root` to a separate path
        // prefix; the 5xx happens on the *remapped* `versions.json`.
        // We want to pin that `versions_async`'s 404→empty-stream
        // downgrade does NOT extend to 5xx on the remapped root: a
        // broken per-project endpoint must propagate as a hard error so
        // the caller sees the failure rather than getting an empty
        // version list.
        let mut server = mockito::Server::new();

        let config_body = format!(r#"{{"index_root":"{}/index/"}}"#, server.url());
        let config_mock = mock_json_get(&mut server, "/sysand-index-config.json", config_body);

        // Remap target returns 503 — a transient server error that is
        // indistinguishable from a misconfiguration and therefore
        // cannot be silently swallowed.
        let remap_mock = server
            .mock("GET", "/index/admin/proj0/versions.json")
            .with_status(503)
            .expect(1)
            .create();

        let env = test_env_sync_discovery(&server)?;

        let err = env
            .versions(purl("admin/proj0"))
            .expect_err("5xx on remapped versions.json must be a hard error");
        let text = format!("{err:?}");
        assert!(
            text.contains("BadHttpStatus") && text.contains("503"),
            "expected BadHttpStatus(503) on remapped versions.json, got: {text}"
        );

        config_mock.assert();
        remap_mock.assert();

        Ok(())
    }

    #[test]
    fn discovery_follows_redirect() -> Result<(), Box<dyn std::error::Error>> {
        // A successful discovery + subsequent versions read proves the
        // redirect was followed and the body was used.
        let mut server = mockito::Server::new();

        let redirect_mock = server
            .mock("GET", "/sysand-index-config.json")
            .with_status(302)
            .with_header("location", "/actual-index-config.json")
            .expect(1)
            .create();

        let target_mock = mock_json_get(&mut server, "/actual-index-config.json", r#"{}"#);

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("1.0.0", "[]")]),
        );

        let env = test_env_sync_discovery(&server)?;

        let versions: Vec<_> = env
            .versions(purl("admin/proj0"))?
            .collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["1.0.0"]);

        redirect_mock.assert();
        target_mock.assert();
        versions_mock.assert();

        Ok(())
    }

    #[test]
    fn discovery_unknown_fields_silently_ignored() -> Result<(), Box<dyn std::error::Error>> {
        // Discovery-document forward-compat case.
        let mut server = mockito::Server::new();

        let config_mock = mock_json_get(
            &mut server,
            "/sysand-index-config.json",
            r#"{"unknown_future_field":"ignore-me","v2_capabilities":["x","y"]}"#,
        );

        let versions_mock = mock_json_get(
            &mut server,
            "/admin/proj0/versions.json",
            versions_json_body([("1.0.0", "[]")]),
        );

        let env = test_env_sync_discovery(&server)?;

        let versions: Vec<_> = env
            .versions(purl("admin/proj0"))?
            .collect::<Result<_, _>>()?;
        assert_eq!(versions, vec!["1.0.0"]);

        config_mock.assert();
        versions_mock.assert();

        Ok(())
    }
}
