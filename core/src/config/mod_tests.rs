// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use url::Url;

use crate::{
    config::{Config, ConfigProject, Index},
    lock::Source,
};

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
            sources: vec![Source::LocalSrc {
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
            Url::parse("http://www.extra-index.com").unwrap(),
            Url::parse("http://www.index.com").unwrap(),
            Url::parse("http://www.default.com").unwrap(),
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
            Url::parse("http://www.extra-index.com").unwrap(),
            Url::parse("http://www.index.com").unwrap(),
            Url::parse("http://www.config-default.com").unwrap(),
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
            Url::parse("http://www.extra-index.com").unwrap(),
            Url::parse("http://www.index.com").unwrap(),
            Url::parse("http://www.new-default.com").unwrap(),
        ]
    );
}
