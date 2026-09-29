// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use url::Url;

use crate::config::{Config, ConfigProject, Index, OverrideSource};
use crate::index_location::IndexLocation;

fn iri(iri: &str) -> Iri<String> {
    Iri::parse(iri).unwrap().into()
}

fn loc(url: &str) -> IndexLocation {
    IndexLocation::parse(url).unwrap()
}

#[test]
fn default_config() {
    let config = Config::default();

    assert_eq!(config.indexes, vec![]);
    assert_eq!(config.projects, vec![]);
}

#[test]
fn merge() {
    let mut defaults = Config::default();
    let config = Config {
        indexes: vec![Index::new_url(loc("http://www.example.com"))],
        projects: vec![ConfigProject {
            identifiers: vec![iri("urn:kpar:test")],
            sources: vec![OverrideSource::LocalSrc {
                src_path: "./path/to project".into(),
            }],
        }],
        // auth: None,
    };
    defaults.merge(config.clone());

    assert_eq!(defaults, config);
}

#[test]
fn index_urls_without_default() {
    let config = Config {
        indexes: vec![Index::new_url(loc("http://www.index.com"))],
        ..Default::default()
    };
    let index = vec![loc("http://www.extra-index.com")];
    let default_urls = vec![loc("http://www.default.com")];
    let default_override_urls = vec![];

    let index_urls = config.index_urls(index, default_urls, default_override_urls);

    assert_eq!(
        index_urls,
        vec![
            IndexLocation::Root(Url::parse("http://www.extra-index.com").unwrap()),
            IndexLocation::Root(Url::parse("http://www.index.com").unwrap()),
            IndexLocation::Root(Url::parse("http://www.default.com").unwrap()),
        ]
    );
}

#[test]
fn index_urls_with_default() {
    let config = Config {
        indexes: vec![
            Index {
                default: Some(true),
                ..Index::new_url(loc("http://www.config-default.com"))
            },
            Index::new_url(loc("http://www.index.com")),
        ],
        ..Default::default()
    };
    let index = vec![loc("http://www.extra-index.com")];
    let default_urls = vec![loc("http://www.default.com")];
    let default_override_urls = vec![];

    let index_urls = config.index_urls(index, default_urls, default_override_urls);

    assert_eq!(
        index_urls,
        vec![
            IndexLocation::Root(Url::parse("http://www.extra-index.com").unwrap()),
            IndexLocation::Root(Url::parse("http://www.index.com").unwrap()),
            IndexLocation::Root(Url::parse("http://www.config-default.com").unwrap()),
        ]
    );
}

#[test]
fn index_urls_with_override() {
    let config = Config {
        indexes: vec![
            Index {
                default: Some(true),
                ..Index::new_url(loc("http://www.config-default.com"))
            },
            Index::new_url(loc("http://www.index.com")),
        ],
        ..Default::default()
    };
    let index = vec![loc("http://www.extra-index.com")];
    let default_urls = vec![loc("http://www.default.com")];
    let default_override_urls = vec![loc("http://www.new-default.com")];

    let index_urls = config.index_urls(index, default_urls, default_override_urls);

    assert_eq!(
        index_urls,
        vec![
            IndexLocation::Root(Url::parse("http://www.extra-index.com").unwrap()),
            IndexLocation::Root(Url::parse("http://www.index.com").unwrap()),
            IndexLocation::Root(Url::parse("http://www.new-default.com").unwrap()),
        ]
    );
}

#[test]
fn index_urls_accepts_templates_everywhere() {
    // URL templates are valid wherever an index URL can be configured:
    // `--index` values, `sysand.toml` `[[index]]` entries, and default
    // overrides all funnel through the same parser.
    let config = Config {
        indexes: vec![Index::new_url(loc(
            "https://gitlab.com/api/v4/projects/123/repository/files/{path}/raw?ref=main",
        ))],
        ..Default::default()
    };
    let index = vec![loc("https://example.org/raw/{path_raw}?ref=main")];
    let default_urls = vec![];
    let default_override_urls = vec![loc("https://other.example/files/{path}/x")];

    let index_urls = config.index_urls(index, default_urls, default_override_urls);

    assert_eq!(
        index_urls
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![
            "https://example.org/raw/{path_raw}?ref=main",
            "https://gitlab.com/api/v4/projects/123/repository/files/{path}/raw?ref=main",
            "https://other.example/files/{path}/x",
        ]
    );
    assert!(
        index_urls
            .iter()
            .all(|l| matches!(l, IndexLocation::Template(_)))
    );
}

#[test]
fn index_url_with_bad_template_is_rejected_on_load() {
    let err = toml::from_str::<Config>(
        r#"
[[index]]
url = "https://example.org/files/{file}/raw"
"#,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("unknown placeholder `{file}`"),
        "{err}"
    );
}

/// The index-location error itself redacts the password, but the TOML
/// error quotes the offending source line, so the password does appear in
/// the message. This is not ideal, but acceptable, as unlike an environment
/// variable, a config file is not expected to be secret in e.g. CI
#[test]
fn index_url_with_password_is_rejected_on_load_and_quoted_in_the_toml_snippet() {
    let err = toml::from_str::<Config>(
        r#"
[[index]]
url = "https://user:hunter2@example.org/"
"#,
    )
    .unwrap_err();
    let err = err.to_string();
    assert!(
        err.contains("index URL `https://<redacted>@example.org/` includes username or password"),
        "{err}"
    );
    assert!(
        err.contains(r#"url = "https://user:hunter2@example.org/""#),
        "{err}"
    );
}

#[test]
fn default_index_locations_returns_only_defaults() {
    let config = Config {
        indexes: vec![
            Index {
                default: Some(true),
                ..Index::new_url(loc("http://www.config-default.com"))
            },
            Index::new_url(loc("http://www.not-default.com")),
            Index {
                default: Some(false),
                ..Index::new_url(loc("http://www.explicitly-not-default.com"))
            },
        ],
        ..Default::default()
    };

    assert_eq!(
        config.default_index_locations(),
        vec![IndexLocation::Root(
            Url::parse("http://www.config-default.com").unwrap()
        )]
    );
}

#[test]
fn index_url_round_trips_through_toml() {
    let config = Config {
        indexes: vec![Index::new_url(loc(
            "https://gitlab.com/api/v4/projects/123/repository/files/{path}/raw?ref=main",
        ))],
        ..Default::default()
    };
    let text = toml::to_string(&config).unwrap();
    assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
}
