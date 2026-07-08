// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use url::Url;

use crate::config::{Config, ConfigProject, Index, OverrideSource};
use crate::index_location::IndexLocation;

#[test]
fn default_config() {
    let config = Config::default();

    assert_eq!(config.indexes, vec![]);
    assert_eq!(config.projects, vec![]);
}

#[test]
fn default_index() {
    let index = Index::default();

    assert_eq!(index.name, None);
    assert_eq!(index.url, "");
    // assert_eq!(index.explicit, None);
    assert_eq!(index.default, None);
}

#[test]
fn merge() {
    let mut defaults = Config::default();
    let config = Config {
        indexes: vec![Index {
            url: "http://www.example.com".to_string(),
            ..Default::default()
        }],
        projects: vec![ConfigProject {
            identifiers: vec!["urn:kpar:test".to_string()],
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
        indexes: vec![Index {
            url: "http://www.index.com".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let index = vec!["http://www.extra-index.com".to_string()];
    let default_urls = vec!["http://www.default.com".to_string()];
    let default_override_urls = vec![];

    let index_urls = config
        .index_urls(index, default_urls, default_override_urls)
        .unwrap();

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
                url: "http://www.config-default.com".to_string(),
                default: Some(true),
                ..Default::default()
            },
            Index {
                url: "http://www.index.com".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let index = vec!["http://www.extra-index.com".to_string()];
    let default_urls = vec!["http://www.default.com".to_string()];
    let default_override_urls = vec![];

    let index_urls = config
        .index_urls(index, default_urls, default_override_urls)
        .unwrap();

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
                url: "http://www.config-default.com".to_string(),
                default: Some(true),
                ..Default::default()
            },
            Index {
                url: "http://www.index.com".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let index = vec!["http://www.extra-index.com".to_string()];
    let default_urls = vec!["http://www.default.com".to_string()];
    let default_override_urls = vec!["http://www.new-default.com".to_string()];

    let index_urls = config
        .index_urls(index, default_urls, default_override_urls)
        .unwrap();

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
        indexes: vec![Index {
            url: "https://gitlab.com/api/v4/projects/123/repository/files/{path}/raw?ref=main"
                .to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let index = vec!["https://example.org/raw/{path_raw}?ref=main".to_string()];
    let default_urls = vec![];
    let default_override_urls = vec!["https://other.example/files/{path}/x".to_string()];

    let index_urls = config
        .index_urls(index, default_urls, default_override_urls)
        .unwrap();

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
fn index_urls_rejects_bad_template() {
    let config = Config::default();
    let err = config
        .index_urls(
            vec!["https://example.org/files/{file}/raw".to_string()],
            vec![],
            vec![],
        )
        .unwrap_err();
    assert!(err.to_string().contains("unknown placeholder `{file}`"));
}
