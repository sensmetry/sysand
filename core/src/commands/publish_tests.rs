// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::{
    AllowedMetamodelKind, PublishBearerConflict, PublishBearerProvenance, PublishError,
    PublishPreparation, SelectedPublishBearer, TrustedPublishingEnvironment, TrustedPublishingMode,
    build_upload_url, check_metamodel, check_usage, do_publish, error_body_to_string,
    map_publish_response, resolve_publish_bearer, resolve_publish_bearer_from_config,
    stored_bearer_clearly_expired, validate_api_root_url_shape,
};
use crate::{
    auth::{EnvBearerAuth, ForceBearerAuth, GlobMap, GlobMapBuilder, StoredBearerAuth},
    index_location::IndexLocation,
    model::InterchangeProjectUsageRaw,
    resolve::net_utils::create_reqwest_client,
};
use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use mockito::Matcher;
use std::assert_matches;
use std::sync::Arc;
use url::Url;

fn runtime() -> Arc<tokio::runtime::Runtime> {
    Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap(),
    )
}

fn empty_sources() -> GlobMap<EnvBearerAuth> {
    GlobMap::default()
}

fn env_sources(entries: &[(&str, &str)]) -> GlobMap<EnvBearerAuth> {
    env_sources_labeled(
        &entries
            .iter()
            .map(|(pattern, token)| (*pattern, *token, "ENVIDX"))
            .collect::<Vec<_>>(),
    )
}

fn env_sources_labeled(entries: &[(&str, &str, &str)]) -> GlobMap<EnvBearerAuth> {
    let mut builder = GlobMapBuilder::new();
    for (pattern, token, label) in entries {
        builder.add(
            *pattern,
            EnvBearerAuth {
                auth: ForceBearerAuth::new(*token),
                label: (*label).to_string(),
            },
        );
    }
    builder.build().unwrap()
}

const STORED_KEY: &str = "https://example.org/";

fn stored_sources(entries: &[(&str, &str)]) -> GlobMap<StoredBearerAuth> {
    stored_sources_expiring(entries, None)
}

fn stored_sources_expiring(
    entries: &[(&str, &str)],
    expires_at: Option<DateTime<Utc>>,
) -> GlobMap<StoredBearerAuth> {
    let mut builder = GlobMapBuilder::new();
    for (pattern, token) in entries {
        builder.add(
            *pattern,
            StoredBearerAuth::new(
                ForceBearerAuth::new(*token),
                STORED_KEY.to_string(),
                expires_at,
            ),
        );
    }
    builder.build().unwrap()
}

fn stored_sources_keyed(entries: &[(&str, &str, &str)]) -> GlobMap<StoredBearerAuth> {
    let mut builder = GlobMapBuilder::new();
    for (pattern, token, key) in entries {
        builder.add(
            *pattern,
            StoredBearerAuth::new(ForceBearerAuth::new(*token), (*key).to_string(), None),
        );
    }
    builder.build().unwrap()
}

/// A stored-credential provider for paths that must never consult the
/// store (env matched, or the flow errors before configured credentials).
fn stored_unreached() -> GlobMap<StoredBearerAuth> {
    panic!("stored credentials must not be read on this path");
}

fn gitlab_env(token: &str) -> TrustedPublishingEnvironment {
    TrustedPublishingEnvironment::new(None, None, Some(token.to_owned()))
}

fn github_env(token: &str, url: &str) -> TrustedPublishingEnvironment {
    TrustedPublishingEnvironment::new(Some(token.to_owned()), Some(url.to_owned()), None)
}

#[test]
fn build_upload_url_appends_endpoint_path() {
    // `build_upload_url` takes the resolved `api_root` (already normalized
    // by discovery to end with `/`), not the discovery root. Well-known
    // discovery has already chosen `api_root` — this helper just composes
    // `v1/upload` onto it.
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org").unwrap()).as_str(),
        "https://example.org/v1/upload"
    );
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/").unwrap()).as_str(),
        "https://example.org/v1/upload"
    );
    // A resolved `api_root` with a sub-path already carries its trailing
    // slash, so the endpoint appends rather than replacing `api`.
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/api/").unwrap()).as_str(),
        "https://example.org/api/v1/upload"
    );
}

#[test]
fn resolve_publish_bearer_auto_uses_bearer_when_trusted_publishing_unavailable() {
    let api_root = Url::parse("https://example.org/api/").unwrap();
    let map = env_sources(&[("https://example.org/api/**", "explicit-token")]);
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Auto,
        &TrustedPublishingEnvironment::new(None, None, None),
        &client,
        &runtime,
    )
    .expect("explicit bearer should be selected when trusted publishing is unavailable");
}

#[test]
fn resolve_publish_bearer_never_rejects_ambiguous_bearer() {
    let api_root = Url::parse("https://example.org/api/").unwrap();
    let map = env_sources(&[
        ("https://example.org/**", "broad-token"),
        ("https://example.org/api/**", "specific-token"),
    ]);
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    let err = resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Never,
        &gitlab_env("gitlab-oidc-token"),
        &client,
        &runtime,
    )
    .unwrap_err();

    // Both entries share the ENVIDX label, so the conflict names the one
    // variable once.
    assert_matches!(
        err,
        PublishError::AmbiguousPublishBearer {
            conflict: PublishBearerConflict::Env { ref variables },
            ..
        } if variables == &["SYSAND_CRED_ENVIDX"]
    );
}

#[test]
fn publish_bearer_env_match_wins_without_reading_stored_credentials() {
    // With an env match the stored-credential provider must not run at
    // all (never read the keyring when env already works).
    let upload_url = Url::parse("https://example.org/api/v1/upload").unwrap();
    let env = env_sources(&[("https://example.org/api/**", "env-token")]);

    let selected = resolve_publish_bearer_from_config(&env, stored_unreached, &upload_url).unwrap();

    assert_eq!(selected.auth, ForceBearerAuth::new("env-token"));
    assert_eq!(
        selected.provenance,
        PublishBearerProvenance::Env {
            label: "ENVIDX".to_string()
        }
    );
}

#[test]
fn publish_bearer_falls_through_to_keyring_when_env_has_no_match() {
    let upload_url = Url::parse("https://example.org/api/v1/upload").unwrap();
    let env = env_sources(&[("https://other.example.com/**", "env-token")]);

    let selected = resolve_publish_bearer_from_config(
        &env,
        || stored_sources(&[("https://example.org/api/**", "keyring-token")]),
        &upload_url,
    )
    .unwrap();

    assert_eq!(selected.auth, ForceBearerAuth::new("keyring-token"));
    assert_eq!(
        selected.provenance,
        PublishBearerProvenance::Stored {
            key: STORED_KEY.to_string(),
            expires_at: None,
        }
    );
}

#[test]
fn publish_bearer_keyring_identical_token_candidates_collapse() {
    // The typical shape: one stored login whose glob set matches the
    // upload URL through more than one pattern. Same token, so publish
    // selects it instead of reporting ambiguity (the collapse rule itself
    // is pinned per-source in the `select_bearer` tests).
    let upload_url = Url::parse("https://example.org/api/v1/upload").unwrap();

    let selected = resolve_publish_bearer_from_config(
        &empty_sources(),
        || {
            stored_sources(&[
                ("https://example.org/**", "shared-keyring-token"),
                ("https://example.org/api/**", "shared-keyring-token"),
            ])
        },
        &upload_url,
    )
    .unwrap();

    assert_eq!(selected.auth, ForceBearerAuth::new("shared-keyring-token"));
    assert_eq!(
        selected.provenance,
        PublishBearerProvenance::Stored {
            key: STORED_KEY.to_string(),
            expires_at: None,
        }
    );
}

#[test]
fn publish_bearer_env_ambiguity_errors_without_keyring_fallback() {
    // An ambiguous env match must error, not fall back to a unique
    // keyring match, and must not read the stored credentials at all.
    let upload_url = Url::parse("https://example.org/api/v1/upload").unwrap();
    let env = env_sources_labeled(&[
        ("https://example.org/**", "broad-env-token", "BROAD"),
        ("https://example.org/api/**", "specific-env-token", "NARROW"),
    ]);

    let err = resolve_publish_bearer_from_config(&env, stored_unreached, &upload_url).unwrap_err();

    // The conflict names every matching `SYSAND_CRED_<LABEL>` variable.
    assert_matches!(
        err,
        PublishError::AmbiguousPublishBearer {
            conflict: PublishBearerConflict::Env { ref variables },
            ref upload_url,
        } if variables == &["SYSAND_CRED_BROAD", "SYSAND_CRED_NARROW"]
            && upload_url.as_ref() == "https://example.org/api/v1/upload"
    );
    let message = err.to_string();
    assert!(message.contains("SYSAND_CRED_BROAD"), "message: {message}");
    assert!(message.contains("SYSAND_CRED_NARROW"), "message: {message}");
}

#[test]
fn publish_bearer_keyring_ambiguity_errors() {
    let upload_url = Url::parse("https://example.org/api/v1/upload").unwrap();

    let err = resolve_publish_bearer_from_config(
        &empty_sources(),
        || {
            stored_sources_keyed(&[
                (
                    "https://example.org/**",
                    "broad-keyring-token",
                    "https://example.org/",
                ),
                (
                    "https://example.org/api/**",
                    "specific-keyring-token",
                    "https://example.org/api/",
                ),
            ])
        },
        &upload_url,
    )
    .unwrap_err();

    // The conflict names every matching stored login by its key.
    assert_matches!(
        err,
        PublishError::AmbiguousPublishBearer {
            conflict: PublishBearerConflict::Stored { ref keys },
            ref upload_url,
        } if keys == &["https://example.org/", "https://example.org/api/"]
            && upload_url.as_ref() == "https://example.org/api/v1/upload"
    );
    let message = err.to_string();
    assert!(
        message.contains("`https://example.org/`")
            && message.contains("`https://example.org/api/`"),
        "message: {message}"
    );
}

#[test]
fn publish_bearer_no_match_in_any_source_errors() {
    let upload_url = Url::parse("https://example.org/api/v1/upload").unwrap();
    let env = env_sources(&[("https://other.example.com/**", "env-token")]);

    let err = resolve_publish_bearer_from_config(
        &env,
        || stored_sources(&[("https://another.example.com/**", "keyring-token")]),
        &upload_url,
    )
    .unwrap_err();

    assert_matches!(err, PublishError::NoPublishBearer { .. });
    // The core message names no CLI command (the `sysand auth login` hint
    // is added by the frontend); it states the condition and the
    // `SYSAND_CRED_*` environment fallback.
    let message = err.to_string();
    assert!(!message.contains("sysand auth"), "message: {message}");
    assert!(message.contains("SYSAND_CRED_<X>"), "message: {message}");
    assert!(
        message.contains("https://example.org/api/v1/upload"),
        "message: {message}"
    );
}

#[test]
fn trusted_publishing_environment_treats_empty_values_as_unset() {
    let api_root = Url::parse("https://example.org/api/").unwrap();
    let map = empty_sources();
    let env = TrustedPublishingEnvironment::new(
        Some(String::new()),
        Some(String::new()),
        Some(String::new()),
    );
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    let err = resolve_publish_bearer(
        &map,
        GlobMap::default,
        &api_root,
        TrustedPublishingMode::Auto,
        &env,
        &client,
        &runtime,
    )
    .unwrap_err();

    assert_matches!(err, PublishError::NoPublishBearer { .. });
}

#[test]
fn resolve_publish_bearer_auto_rejects_multiple_supported_providers() {
    let api_root = Url::parse("https://example.org/api/").unwrap();
    let map = empty_sources();
    let env = TrustedPublishingEnvironment::new(
        Some("github-request-token".to_owned()),
        Some("https://github.example/oidc".to_owned()),
        Some("gitlab-oidc-token".to_owned()),
    );
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    let err = resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Auto,
        &env,
        &client,
        &runtime,
    )
    .unwrap_err();

    assert_matches!(err, PublishError::MultipleTrustedPublishingProviders);
}

#[test]
fn resolve_publish_bearer_always_reports_partial_github_env() {
    let api_root = Url::parse("https://example.org/api/").unwrap();
    let map = empty_sources();
    let env =
        TrustedPublishingEnvironment::new(Some("github-request-token".to_owned()), None, None);
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    let err = resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Always,
        &env,
        &client,
        &runtime,
    )
    .unwrap_err();

    assert_matches!(
        err,
        PublishError::MissingTrustedPublishingEnvironment { provider, .. }
            if provider == super::TrustedPublishingProvider::Github
    );
}

#[test]
fn resolve_publish_bearer_always_requires_supported_env() {
    let api_root = Url::parse("https://example.org/api/").unwrap();
    let map = env_sources(&[("https://example.org/api/**", "explicit-token")]);
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    let err = resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Always,
        &TrustedPublishingEnvironment::new(None, None, None),
        &client,
        &runtime,
    )
    .unwrap_err();

    assert_matches!(err, PublishError::TrustedPublishingUnavailable);
}

#[test]
fn resolve_publish_bearer_invalid_github_url_makes_no_exchange_request() {
    let mut server = mockito::Server::new();
    let exchange_mock = server.mock("POST", "/api/v1/oidc/token").expect(0).create();
    let api_root = Url::parse(&format!("{}/api/", server.url())).unwrap();
    let map = empty_sources();
    let env = github_env("github-request-token", "not a url");
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    let err = resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Always,
        &env,
        &client,
        &runtime,
    )
    .unwrap_err();

    assert_matches!(err, PublishError::InvalidGithubOidcRequestUrl { .. });
    exchange_mock.assert();
}

#[test]
fn resolve_publish_bearer_github_preserves_existing_oidc_query_params() {
    let mut index_server = mockito::Server::new();
    let exchange_mock = index_server
        .mock("POST", "/api/v1/oidc/token")
        .match_header("content-type", "application/json")
        .match_body(Matcher::JsonString(
            r#"{"token":"github-oidc-token"}"#.to_owned(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"token":"index-token"}"#)
        .expect(1)
        .create();
    let api_root = Url::parse(&format!("{}/api/", index_server.url())).unwrap();

    let mut github_server = mockito::Server::new();
    let github_mock = github_server
        .mock("GET", "/oidc")
        .match_header("authorization", "bearer github-request-token")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("existing".to_owned(), "1".to_owned()),
            Matcher::UrlEncoded("audience".to_owned(), "sysand".to_owned()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"value":"github-oidc-token"}"#)
        .expect(1)
        .create();

    let map = empty_sources();
    let env = github_env(
        "github-request-token",
        &format!("{}/oidc?existing=1", github_server.url()),
    );
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Auto,
        &env,
        &client,
        &runtime,
    )
    .expect("GitHub trusted publishing should resolve a bearer token");

    github_mock.assert();
    exchange_mock.assert();
}

#[test]
fn resolve_publish_bearer_gitlab_exchange_success() {
    let mut server = mockito::Server::new();
    let exchange_mock = server
        .mock("POST", "/api/v1/oidc/token")
        .match_header("content-type", "application/json")
        .match_body(Matcher::JsonString(
            r#"{"token":"gitlab-oidc-token"}"#.to_owned(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"token":"index-token"}"#)
        .expect(1)
        .create();
    let api_root = Url::parse(&format!("{}/api/", server.url())).unwrap();
    let map = empty_sources();
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Auto,
        &gitlab_env("gitlab-oidc-token"),
        &client,
        &runtime,
    )
    .expect("GitLab trusted publishing should resolve a bearer token");

    exchange_mock.assert();
}

#[test]
fn resolve_publish_bearer_exchange_non_success_errors() {
    let mut server = mockito::Server::new();
    let exchange_mock = server
        .mock("POST", "/api/v1/oidc/token")
        .with_status(403)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error": "No matching trusted publisher found"}"#)
        .expect(1)
        .create();
    let api_root = Url::parse(&format!("{}/api/", server.url())).unwrap();
    let map = empty_sources();
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    let err = resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Auto,
        &gitlab_env("gitlab-oidc-token"),
        &client,
        &runtime,
    )
    .unwrap_err();

    assert_matches!(
        &err,
        PublishError::TrustedPublishingExchangeHttpStatus { status: 403, detail, .. }
            if detail.as_ref() == "No matching trusted publisher found"
    );
    // The server-provided reason is surfaced in the error's Display output.
    assert!(
        err.to_string()
            .contains("No matching trusted publisher found"),
        "error message should include the server reason: {err}"
    );
    exchange_mock.assert();
}

#[test]
fn resolve_publish_bearer_exchange_malformed_response_errors() {
    let mut server = mockito::Server::new();
    let exchange_mock = server
        .mock("POST", "/api/v1/oidc/token")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"not_token":"index-token"}"#)
        .expect(1)
        .create();
    let api_root = Url::parse(&format!("{}/api/", server.url())).unwrap();
    let map = empty_sources();
    let client = create_reqwest_client().unwrap();
    let runtime = runtime();

    let err = resolve_publish_bearer(
        &map,
        stored_unreached,
        &api_root,
        TrustedPublishingMode::Auto,
        &gitlab_env("gitlab-oidc-token"),
        &client,
        &runtime,
    )
    .unwrap_err();

    assert_matches!(err, PublishError::MissingJsonField { field: "token", .. });
    exchange_mock.assert();
}

#[test]
fn build_upload_url_preserves_percent_encoded_segments() {
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/my%20api/").unwrap()).as_str(),
        "https://example.org/my%20api/v1/upload"
    );
}

#[test]
fn error_body_to_string_trims_text_content() {
    assert_eq!(error_body_to_string(b"  unauthorized\n"), "unauthorized");
}

#[test]
fn error_body_to_string_extracts_error_from_json() {
    assert_eq!(
        error_body_to_string(br#"{"error":"Invalid token"}"#),
        "Invalid token"
    );
}

#[test]
fn error_body_to_string_reports_empty_body() {
    assert_eq!(error_body_to_string(b" \n\t "), "no error details provided");
}

// --- check_metamodel ---

#[test]
fn check_metamodel_accepts_valid_sysml() {
    assert_eq!(
        check_metamodel("https://www.omg.org/spec/SysML/20250201").unwrap(),
        AllowedMetamodelKind::SysML,
    );
}

#[test]
fn check_metamodel_accepts_valid_kerml() {
    assert_eq!(
        check_metamodel("https://www.omg.org/spec/KerML/20250201").unwrap(),
        AllowedMetamodelKind::KerML,
    );
}

#[test]
fn check_metamodel_rejects_unsupported_metamodel() {
    let err = check_metamodel("https://example.com/some-meta").unwrap_err();
    assert_matches!(err, PublishError::UnsupportedMetamodel { .. });
}

#[test]
fn check_metamodel_rejects_invalid_sysml_version() {
    // Valid SysML prefix but non-date version string.
    let err = check_metamodel("https://www.omg.org/spec/SysML/notadate").unwrap_err();
    assert_matches!(err, PublishError::InvalidMetamodelVersion { .. });
}

#[test]
fn check_metamodel_rejects_invalid_kerml_version() {
    // Month 13 is not a valid calendar month.
    let err = check_metamodel("https://www.omg.org/spec/KerML/20251301").unwrap_err();
    assert_matches!(err, PublishError::InvalidMetamodelVersion { .. });
}

// --- check_usage ---

fn usage(resource: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Resource {
        resource: resource.to_string(),
        version_constraint: None,
    }
}

fn usage_with_vc(resource: &str, vc: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Resource {
        resource: resource.to_string(),
        version_constraint: Some(vc.to_string()),
    }
}

#[test]
fn check_usage_accepts_valid_sysand_purl() {
    check_usage(&usage("pkg:sysand/acme/widget")).unwrap();
}

#[test]
fn check_usage_accepts_all_known_std_libs() {
    for resource in [
        "https://www.omg.org/spec/KerML/20250201/Data-Type-Library.kpar",
        "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar",
        "https://www.omg.org/spec/KerML/20250201/Function-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Analysis-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Cause-and-Effect-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Geometry-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Metadata-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Quantities-and-Units-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Requirement-Derivation-Domain-Library.kpar",
    ] {
        check_usage(&usage(resource)).unwrap_or_else(|e| panic!("{resource}: {e}"));
    }
}

#[test]
fn check_usage_rejects_disallowed_usage() {
    // Not a pkg:sysand purl and not a std-lib IRI prefix.
    let err = check_usage(&usage("https://example.com/some/library")).unwrap_err();
    assert_matches!(err, PublishError::DisallowedUsage { .. });
}

#[test]
fn check_usage_rejects_invalid_purl() {
    // pkg:sysand prefix present but name segment is syntactically invalid.
    let err = check_usage(&usage("pkg:sysand/publisher/bad__name")).unwrap_err();
    assert_matches!(err, PublishError::InvalidPurl { .. });
}

#[test]
fn check_usage_rejects_std_lib_with_version_constraint() {
    let err = check_usage(&usage_with_vc(
        "https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar",
        ">=1.0.0",
    ))
    .unwrap_err();
    assert_matches!(err, PublishError::StdWithVersionConstraint { .. });
}

#[test]
fn check_usage_rejects_invalid_std_lib_version() {
    // Valid SysML prefix + known suffix, but invalid date portion.
    let err = check_usage(&usage(
        "https://www.omg.org/spec/SysML/baddate/Systems-Library.kpar",
    ))
    .unwrap_err();
    assert_matches!(err, PublishError::InvalidStdLibVersion { .. });
}

#[test]
fn check_usage_rejects_unknown_std_lib() {
    // Valid SysML prefix, no version constraint, but the suffix is not a known library.
    let err = check_usage(&usage(
        "https://www.omg.org/spec/SysML/20250201/Nonexistent-Library.kpar",
    ))
    .unwrap_err();
    assert_matches!(err, PublishError::UnknownStdLib { .. });
}

fn directory_usage(dir: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Directory {
        dir: dir.to_string(),
        publisher: "acme".to_string(),
        name: "lib".to_string(),
    }
}

#[test]
fn check_usage_rejects_directory_usage() {
    let err = check_usage(&directory_usage("../local-dep")).unwrap_err();
    assert_matches!(err, PublishError::PathUsage { path } if path.as_ref() == "../local-dep");
}

#[test]
fn check_usage_rejects_directory_usage_with_version_constraint() {
    // Directory usages have no version_constraint field, but verify the error
    // is PathUsage regardless of publisher/name values.
    let err = check_usage(&InterchangeProjectUsageRaw::Directory {
        dir: "libs/sub".to_string(),
        publisher: "org".to_string(),
        name: "thing".to_string(),
    })
    .unwrap_err();
    assert_matches!(err, PublishError::PathUsage { path } if path.as_ref() == "libs/sub");
}

// --- map_publish_response ---

#[test]
fn map_publish_response_400_maps_to_bad_request() {
    let err = map_publish_response(
        400,
        b"bad field",
        &PublishBearerProvenance::TrustedPublishing,
    )
    .unwrap_err();
    assert_matches!(err, PublishError::BadRequest(_));
}

#[test]
fn map_publish_response_200_is_ok_not_new_project() {
    let resp =
        map_publish_response(200, b"ok", &PublishBearerProvenance::TrustedPublishing).unwrap();
    assert!(!resp.is_new_project);
    assert_eq!(resp.status, 200);
}

#[test]
fn map_publish_response_201_is_ok_new_project() {
    let resp =
        map_publish_response(201, b"created", &PublishBearerProvenance::TrustedPublishing).unwrap();
    assert!(resp.is_new_project);
    assert_eq!(resp.status, 201);
}

// --- source-named auth failures and the pre-upload expiry stop ---

fn preparation() -> PublishPreparation {
    PublishPreparation {
        norm_publisher: "acme".to_string(),
        norm_name: "widgets".to_string(),
        version: "1.0.0".to_string(),
        metadata: "{}".to_string(),
        kpar_bytes: Bytes::from_static(b"not a real kpar"),
    }
}

/// Run `do_publish` against `server`'s `/api/v1/upload` with the given
/// selected bearer.
fn publish_to(
    server: &mockito::Server,
    bearer: SelectedPublishBearer,
) -> Result<super::PublishResponse, PublishError> {
    let discovery_root = IndexLocation::parse(&server.url()).unwrap();
    let api_root = Url::parse(&format!("{}/api/", server.url())).unwrap();
    let client = create_reqwest_client().unwrap();
    do_publish(
        preparation(),
        discovery_root,
        api_root,
        bearer,
        client,
        runtime(),
    )
}

fn env_bearer(label: &str) -> SelectedPublishBearer {
    SelectedPublishBearer {
        auth: ForceBearerAuth::new("env-token"),
        provenance: PublishBearerProvenance::Env {
            label: label.to_string(),
        },
    }
}

fn stored_bearer(expires_at: Option<DateTime<Utc>>) -> SelectedPublishBearer {
    SelectedPublishBearer {
        auth: ForceBearerAuth::new("stored-token"),
        provenance: PublishBearerProvenance::Stored {
            key: STORED_KEY.to_string(),
            expires_at,
        },
    }
}

fn upload_mock(server: &mut mockito::Server, status: usize, body: &str) -> mockito::Mock {
    server
        .mock("POST", "/api/v1/upload")
        .with_status(status)
        .with_body(body)
        .expect(1)
        .create()
}

#[test]
fn publish_env_auth_failure_names_the_env_var() {
    let mut server = mockito::Server::new();
    let mock = upload_mock(&mut server, 401, "unauthorized");

    let err = publish_to(&server, env_bearer("TEAMIDX")).unwrap_err();

    let message = err.to_string();
    assert!(message.contains("HTTP 401"), "message: {message}");
    assert!(
        message.contains("`SYSAND_CRED_TEAMIDX_BEARER_TOKEN`"),
        "message: {message}"
    );
    // Env shadows stored credentials, so re-authenticating must not be the
    // suggested fix. The core message names no CLI command.
    assert!(message.contains("cannot replace it"), "message: {message}");
    assert!(!message.contains("sysand auth"), "message: {message}");
    mock.assert();
}

#[test]
fn publish_stored_auth_failure_names_the_credential() {
    let mut server = mockito::Server::new();
    let mock = upload_mock(&mut server, 401, "unauthorized");

    let err = publish_to(&server, stored_bearer(None)).unwrap_err();

    let message = err.to_string();
    // The core message states the source only; the frontend adds the
    // `sysand auth login` remediation.
    assert!(
        message.contains(&format!("the stored credential for `{STORED_KEY}`")),
        "message: {message}"
    );
    assert!(!message.contains("sysand auth"), "message: {message}");
    assert!(!message.contains("SYSAND_CRED_"), "message: {message}");
    mock.assert();
}

#[test]
fn publish_403_points_at_the_credential_subject() {
    // 403 is authorization, not authentication: the message points at the
    // credential's subject, catching a wrong-project token, without naming
    // a CLI command.
    let mut server = mockito::Server::new();
    let mock = upload_mock(&mut server, 403, "forbidden");

    let err = publish_to(&server, stored_bearer(None)).unwrap_err();

    let message = err.to_string();
    assert!(message.contains("HTTP 403"), "message: {message}");
    assert!(
        message.contains("a token for a different project"),
        "message: {message}"
    );
    assert!(!message.contains("sysand auth"), "message: {message}");
    mock.assert();
}

#[test]
fn publish_trusted_publishing_auth_failure_stays_generic() {
    // A trusted-publishing token has no user-fixable source: neither env
    // nor stored-login remediation applies.
    let mut server = mockito::Server::new();
    let _mock = upload_mock(&mut server, 401, "unauthorized");

    let bearer = SelectedPublishBearer {
        auth: ForceBearerAuth::new("exchanged-token"),
        provenance: PublishBearerProvenance::TrustedPublishing,
    };
    let err = publish_to(&server, bearer).unwrap_err();

    assert_matches!(&err, PublishError::AuthError(detail) if detail == "unauthorized");
}

#[test]
fn publish_stops_before_upload_when_the_stored_bearer_is_expired() {
    let mut server = mockito::Server::new();
    let mock = server.mock("POST", "/api/v1/upload").expect(0).create();

    let expires_at = Utc::now() - Duration::days(1);
    let err = publish_to(&server, stored_bearer(Some(expires_at))).unwrap_err();

    assert_matches!(
        &err,
        PublishError::StoredCredentialExpired { key, .. } if key == STORED_KEY
    );
    let message = err.to_string();
    // The core message states the condition without naming a CLI command;
    // the frontend adds the `sysand auth login` remediation.
    assert!(message.contains("expired at"), "message: {message}");
    assert!(!message.contains("sysand auth"), "message: {message}");
    // The archive was never uploaded.
    mock.assert();
}

#[test]
fn publish_uploads_normally_without_a_known_expiry() {
    // An env bearer carries no expiry and must never trip the stop.
    let mut server = mockito::Server::new();
    let mock = upload_mock(&mut server, 201, "created");

    let response = publish_to(&server, env_bearer("TEAMIDX")).unwrap();

    assert!(response.is_new_project);
    mock.assert();
}

#[test]
fn stored_bearer_clearly_expired_allows_a_skew_margin() {
    let now = Utc::now();
    // Within the one-hour margin: a skewed client clock must not false-trip
    // and refuse a token the server would accept; the server's 401 stays the
    // authority.
    assert!(!stored_bearer_clearly_expired(
        now - Duration::minutes(30),
        now
    ));
    // Well past the margin: skip uploading with a known-dead token.
    assert!(stored_bearer_clearly_expired(now - Duration::hours(2), now));
    // Not yet expired.
    assert!(!stored_bearer_clearly_expired(
        now + Duration::hours(1),
        now
    ));
    // Exactly at the one-hour boundary: the comparison is strict (`>`), so
    // this is still within the margin; one second past it flips to expired.
    // Pins the boundary against an accidental `>=`.
    assert!(!stored_bearer_clearly_expired(
        now - Duration::hours(1),
        now
    ));
    assert!(stored_bearer_clearly_expired(
        now - Duration::hours(1) - Duration::seconds(1),
        now
    ));
}

// --- validate_api_root_url_shape ---

#[test]
fn validate_api_root_rejects_non_http_scheme() {
    let url = Url::parse("ftp://example.org").unwrap();
    let err = validate_api_root_url_shape(&url).unwrap_err();
    assert_matches!(err, PublishError::InvalidApiRoot { .. });
}

#[test]
fn validate_api_root_rejects_query_and_fragment() {
    for url in [
        "https://example.org/index?x=1",
        "https://example.org/index#frag",
        "https://example.org/index?x=1#frag",
    ] {
        let err = validate_api_root_url_shape(&Url::parse(url).unwrap()).unwrap_err();
        assert_matches!(err, PublishError::InvalidApiRoot { .. });
    }
}

#[test]
fn validate_api_root_rejects_non_hierarchical_url() {
    let err =
        validate_api_root_url_shape(&Url::parse("mailto:test@example.org").unwrap()).unwrap_err();
    assert_matches!(err, PublishError::InvalidApiRoot { .. });
}

#[test]
fn validate_api_root_rejects_userinfo() {
    for raw in [
        "https://user@example.org/api/",
        "https://user:password@example.org/api/",
    ] {
        let err = validate_api_root_url_shape(&Url::parse(raw).unwrap()).unwrap_err();
        assert_matches!(err, PublishError::InvalidApiRoot { .. });
    }
}

// --- prepare_publish_payload error cases ---

mod prepare_publish {
    use crate::utils::{RelativeUnixPathError, sha256_lowercase_hex};
    use std::assert_matches;

    use super::super::prepare_publish_payload;
    use super::PublishError;
    use camino::Utf8PathBuf;
    use camino_tempfile::NamedUtf8TempFile;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn deflate() -> SimpleFileOptions {
        SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::DEFAULT)
    }

    fn stored() -> SimpleFileOptions {
        SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::DEFAULT)
    }

    /// Write a ZIP with the given entries to a NamedTempFile; keep the file
    /// alive by returning it alongside the path.
    fn write_zip(entries: &[(&str, &[u8], SimpleFileOptions)]) -> (NamedUtf8TempFile, Utf8PathBuf) {
        let tmp = NamedUtf8TempFile::new().unwrap();
        let path = Utf8PathBuf::from(tmp.path());
        {
            let f = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            for (name, content, opts) in entries {
                zip.start_file(*name, *opts).unwrap();
                zip.write_all(content).unwrap();
            }
            zip.finish().unwrap();
        }
        (tmp, path)
    }

    /// Minimal `.project.json` that passes all info-level checks up to the
    /// archive loop. Caller can override individual fields in the JSON.
    fn project_json(publisher: &str, name: &str, version: &str, license: &str) -> Vec<u8> {
        format!(
            r#"{{"name":"{name}","publisher":"{publisher}","version":"{version}","license":"{license}"}}"#
        )
        .into_bytes()
    }

    fn base_project() -> Vec<u8> {
        project_json("test-pub", "test-pkg", "1.0.0", "MIT")
    }

    /// `.meta.json` with a single source file, correct checksum, and valid
    /// SysML metamodel. `file_content` is the bytes that will be written to
    /// the archive for `file_name`.
    fn meta_json_with_file(file_name: &str, file_content: &[u8], symbol: &str) -> Vec<u8> {
        let cksum = sha256_lowercase_hex(file_content);
        format!(
            r#"{{"index":{{"{symbol}":"{file_name}"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"{file_name}":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        )
        .into_bytes()
    }

    /// `.meta.json` with no source files (empty index, no checksum).
    fn meta_json_empty() -> Vec<u8> {
        br#"{"index":{},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201"}"#.to_vec()
    }

    /// Base entries that pass every check up to (but not including) the
    /// archive-file loop. Archive loop errors can be triggered by appending
    /// one more "bad" entry.
    fn pre_loop_entries() -> (Vec<u8>, Vec<u8>) {
        (base_project(), meta_json_empty())
    }

    /// Complete set of archive entries for a fully-valid kpar with one source
    /// file `test.sysml` containing `content`.
    fn valid_entries(sysml_content: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let meta = meta_json_with_file("test.sysml", sysml_content, "Test");
        (base_project(), meta)
    }

    #[test]
    fn kpar_read_rejects_non_zip_file() {
        let tmp = NamedUtf8TempFile::new().unwrap();
        std::fs::write(tmp.path(), b"this is not a zip file").unwrap();
        let err = prepare_publish_payload(tmp.path()).expect_err("expected Err");
        assert_matches!(err, PublishError::KparRead(..));
    }

    #[test]
    fn kpar_read_rejects_zip_without_project_json() {
        // A valid ZIP but containing no .project.json — guess_root fails.
        let (_tmp, path) = write_zip(&[("unrelated.txt", b"hello", deflate())]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::KparRead(..));
    }

    #[test]
    fn project_not_at_root() {
        // .project.json is inside a subdirectory; publish requires root placement.
        let (_tmp, path) =
            write_zip(&[("subdir/.project.json", base_project().as_slice(), deflate())]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::ProjectNotAtRoot { .. });
    }

    #[test]
    fn missing_meta() {
        let (_tmp, path) = write_zip(&[(".project.json", base_project().as_slice(), deflate())]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::MissingMeta);
    }

    #[test]
    fn info_meta_validation_project_bad_semver() {
        let bad_project = project_json("test-pub", "test-pkg", "not-semver", "MIT");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad_project.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(
            err,
            PublishError::InfoMetaValidation {
                name: "project",
                ..
            }
        );
    }

    #[test]
    fn info_meta_validation_meta_bad_checksum_alg() {
        // Algorithm field is not one of the recognised values.
        let bad_meta = br#"{"index":{"Sym":"f.sysml"},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{"f.sysml":{"value":"aa","algorithm":"NOTAKNOWNALG"}}}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", bad_meta, deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::InfoMetaValidation { name: "meta", .. });
    }

    #[test]
    fn missing_publisher() {
        let no_pub = br#"{"name":"test-pkg","version":"1.0.0","license":"MIT"}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", no_pub, deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::MissingPublisher);
    }

    #[test]
    fn invalid_publisher() {
        let bad = project_json("bad__pub", "test-pkg", "1.0.0", "MIT");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::InvalidPublisher(..));
    }

    #[test]
    fn invalid_name() {
        let bad = project_json("test-pub", "bad__name", "1.0.0", "MIT");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::InvalidName(..));
    }

    #[test]
    fn version_build_metadata() {
        let bad = project_json("test-pub", "test-pkg", "1.0.0+build", "MIT");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::VersionBuildMetadata { .. });
    }

    #[test]
    fn missing_license() {
        let no_lic = br#"{"name":"test-pkg","publisher":"test-pub","version":"1.0.0"}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", no_lic, deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::MissingLicense);
    }

    #[test]
    fn invalid_license() {
        let bad = project_json("test-pub", "test-pkg", "1.0.0", "NOT-A-LICENSE!!!");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::InvalidLicense { .. });
    }

    #[test]
    fn missing_metamodel() {
        let no_meta = br#"{"index":{},"created":"2025-01-01T00:00:00Z"}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", no_meta, deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::MissingMetamodel);
    }

    // Each test below uses valid .project.json + .meta.json and appends one
    // "bad" file entry; the archive loop fires before any checksum check.

    #[test]
    fn exec_in_archive() {
        let (proj, meta) = pre_loop_entries();
        let exec = deflate().unix_permissions(0o100755);
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("test.sysml", b"package Test;", exec),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::ExecInArchive { .. });
    }

    #[test]
    fn unsupported_compression() {
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("test.sysml", b"package Test;", stored()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::UnsupportedCompression { .. });
    }

    #[test]
    fn symlink_in_archive() {
        let (proj, meta) = pre_loop_entries();
        let tmp = NamedUtf8TempFile::new().unwrap();
        {
            let f = std::fs::File::create(tmp.path()).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            zip.start_file(".project.json", deflate()).unwrap();
            zip.write_all(proj.as_slice()).unwrap();
            zip.start_file(".meta.json", deflate()).unwrap();
            zip.write_all(meta.as_slice()).unwrap();
            zip.add_symlink("test.sysml", "target-path", deflate())
                .unwrap();
            zip.finish().unwrap();
        }
        let err = prepare_publish_payload(tmp.path()).expect_err("expected Err");
        assert_matches!(err, PublishError::Symlink { .. });
    }

    #[test]
    fn encrypted_entry() {
        use zip::unstable::write::FileOptionsExt;
        let (proj, meta) = pre_loop_entries();
        let enc_opts = deflate().with_deprecated_encryption(b"secret").unwrap();
        let tmp = NamedUtf8TempFile::new().unwrap();
        {
            let f = std::fs::File::create(tmp.path()).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            zip.start_file(".project.json", deflate()).unwrap();
            zip.write_all(proj.as_slice()).unwrap();
            zip.start_file(".meta.json", deflate()).unwrap();
            zip.write_all(meta.as_slice()).unwrap();
            zip.start_file("test.sysml", enc_opts).unwrap();
            zip.write_all(b"package Test;").unwrap();
            zip.finish().unwrap();
        }
        let err = prepare_publish_payload(tmp.path()).expect_err("expected Err");
        assert_matches!(err, PublishError::Encrypted { .. });
    }

    #[test]
    fn disallowed_path_current_dir_prefix() {
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("./test.sysml", b"package Test;", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(
            err,
            PublishError::InvalidPathInArchive(RelativeUnixPathError::ContainsCurrent { .. })
        );
    }

    #[test]
    fn missing_license_file() {
        // Valid project + meta, archive passes file loop, but LICENSES/MIT.txt
        // is absent.
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::MissingLicenseFile { .. });
    }

    #[test]
    fn missing_checksum() {
        // meta.json has no checksum field at all.
        let meta_no_cksum =
            br#"{"index":{},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201"}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta_no_cksum, deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::MissingChecksum);
    }

    #[test]
    fn empty_checksum() {
        // checksum is present but empty.
        let meta_empty_cksum =
            br#"{"index":{},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{}}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta_empty_cksum, deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::EmptyChecksum);
    }

    #[test]
    fn incorrect_file_format() {
        // checksum references a .kerml file but metamodel is SysML.
        let cksum = sha256_lowercase_hex(b"content");
        let meta = format!(
            r#"{{"index":{{"Sym":"f.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"f.kerml":{{"value":"{cksum}","algorithm":"SHA256"}},"f.sysml":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("f.sysml", b"content", deflate()),
            ("f.kerml", b"content", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::IncorrectFileFormat { .. });
    }

    #[test]
    fn unsupported_file_checksum_type() {
        // SHA1 is a valid algorithm but not SHA256.
        let sha1_val = "a".repeat(40); // 40 hex chars = valid SHA1 length
        let meta = format!(
            r#"{{"index":{{"Sym":"f.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"f.sysml":{{"value":"{sha1_val}","algorithm":"SHA1"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::UnsupportedFileChecksumType { .. });
    }

    #[test]
    fn missing_file() {
        // checksum mentions test.sysml but the archive does not contain it.
        let cksum = sha256_lowercase_hex(b"package Test;");
        let meta = format!(
            r#"{{"index":{{"Test":"test.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"test.sysml":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            // test.sysml intentionally omitted
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::MissingFile { .. });
    }

    #[test]
    fn incorrect_file_checksum() {
        let wrong_cksum = "f".repeat(64); // valid hex length but wrong value
        let meta = format!(
            r#"{{"index":{{"Test":"test.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"test.sysml":{{"value":"{wrong_cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", b"package Test;", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::IncorrectFileChecksum { .. });
    }

    #[test]
    fn nonexistent_symbol_exported() {
        // index claims "Ghost" is defined in test.sysml, but the file only
        // defines the package "Test".
        let content = b"package Test;";
        let cksum = sha256_lowercase_hex(content);
        let meta = format!(
            r#"{{"index":{{"Ghost":"test.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"test.sysml":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", content, deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::NonexistentSymbolExported { .. });
    }

    #[test]
    fn index_fail() {
        // Garbage content in a .sysml file causes extract_symbols to fail.
        let content = b"@@@NOT VALID SYSML@@@";
        let cksum = sha256_lowercase_hex(content);
        let meta = format!(
            r#"{{"index":{{"Sym":"test.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"test.sysml":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", content, deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::IndexFail { .. });
    }

    #[test]
    fn unexpected_file() {
        // A file that is neither in checksum nor a recognised ancillary file.
        let content = b"package Test;";
        let (proj, meta) = valid_entries(content);
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", content, deflate()),
            ("extra.txt", b"surprise", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::UnexpectedFile { .. });
    }

    #[test]
    fn backslash_in_path() {
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("subdir\\file.sysml", b"package Test;", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(
            err,
            PublishError::InvalidPathInArchive(RelativeUnixPathError::ContainsBackslash { .. }),
        );
    }

    #[test]
    fn absolute_path() {
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("/absolute/file.sysml", b"package Test;", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(
            err,
            PublishError::InvalidPathInArchive(RelativeUnixPathError::Absolute { .. }),
        );
    }

    #[test]
    fn double_slash_in_path() {
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("foo//bar.sysml", b"package Test;", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(
            err,
            PublishError::InvalidPathInArchive(RelativeUnixPathError::ContainsDoubleSlash { .. }),
        );
    }

    #[test]
    fn relative_path_parent_dir() {
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("../escape.sysml", b"package Test;", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(
            err,
            PublishError::InvalidPathInArchive(RelativeUnixPathError::ContainsParent { .. })
        );
    }

    #[test]
    fn compressed_dir_entry() {
        // A directory entry (name ends with '/') must use Stored compression.
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("subdir/", b"", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::CompressedDirEntry { .. });
    }

    #[test]
    fn dir_entry_with_stored_passes_archive_loop() {
        // A Stored-compression directory entry must not trigger UnsupportedCompression
        // or CompressedDirEntry; the function should proceed past the archive loop.
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("LICENSES/", b"", stored()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert_matches!(err, PublishError::MissingChecksum);
    }

    #[test]
    fn changelog_md_accepted() {
        let content = b"package Test;";
        let (proj, meta) = valid_entries(content);
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", content, deflate()),
            (
                "CHANGELOG.md",
                b"# Changelog\n\n## 1.0.0\n- initial release",
                deflate(),
            ),
        ]);
        prepare_publish_payload(&path)
            .expect("CHANGELOG.md must not be rejected as UnexpectedFile");
    }

    #[test]
    fn readme_md_accepted() {
        let content = b"package Test;";
        let (proj, meta) = valid_entries(content);
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", content, deflate()),
            ("README.md", b"# My Package", deflate()),
        ]);
        prepare_publish_payload(&path).expect("README.md must not be rejected as UnexpectedFile");
    }

    #[test]
    fn valid_kpar_succeeds() {
        let content = b"package Test;";
        let (proj, meta) = valid_entries(content);
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", content, deflate()),
        ]);
        prepare_publish_payload(&path).expect("fully valid kpar should succeed");
    }
}
