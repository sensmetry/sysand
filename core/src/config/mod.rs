// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{Deserialize, Serialize};
use url::Url;

use crate::lock::Source;

#[cfg(feature = "filesystem")]
pub mod local_fs;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub quiet: Option<bool>,
    pub verbose: Option<bool>,
    #[serde(rename = "index", skip_serializing_if = "Vec::is_empty", default)]
    pub indexes: Vec<Index>,
    #[serde(rename = "project", skip_serializing_if = "Vec::is_empty", default)]
    pub projects: Vec<ConfigProject>,
    // pub auth: Option<Vec<AuthSource>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigProject {
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub identifiers: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub sources: Vec<Source>,
}

impl Config {
    pub fn merge(&mut self, config: Config) {
        let Config {
            quiet,
            verbose,
            mut indexes,
            mut projects,
        } = config;
        self.quiet = self.quiet.or(quiet);
        self.verbose = self.verbose.or(verbose);
        self.indexes.append(&mut indexes);
        self.projects.append(&mut projects);

        // if let Some(auth) = config.auth {
        //     self.auth = Some(auth.clone());
        // }
    }

    pub fn index_urls(
        &self,
        index_urls: Vec<String>,
        default_urls: Vec<String>,
        default_override_urls: Vec<String>,
    ) -> Result<Vec<Url>, url::ParseError> {
        if default_override_urls.is_empty() {
            self.index_urls_no_default_override(index_urls, default_urls)
        } else {
            self.index_urls_with_default_override(index_urls, default_override_urls)
        }
    }

    fn index_urls_no_default_override(
        &self,
        index_urls: Vec<String>,
        default_urls: Vec<String>,
    ) -> Result<Vec<Url>, url::ParseError> {
        let mut indexes = self.indexes.clone();

        indexes.sort_by_key(|i| i.default.unwrap_or(false));

        let has_default = indexes
            .last()
            .and_then(|index| index.default)
            .unwrap_or(false);

        let end = if has_default { vec![] } else { default_urls };

        index_urls
            .iter()
            .map(|url| url.as_str())
            .chain(indexes.iter().map(|i| i.url.as_str()))
            .chain(end.iter().map(|url| url.as_str()))
            .map(Url::parse)
            .collect()
    }

    fn index_urls_with_default_override(
        &self,
        index_urls: Vec<String>,
        default_urls: Vec<String>,
    ) -> Result<Vec<Url>, url::ParseError> {
        index_urls
            .iter()
            .map(|url| url.as_str())
            .chain(
                self.indexes
                    .iter()
                    .filter(|i| !i.default.unwrap_or(false))
                    .map(|i| i.url.as_str()),
            )
            .chain(default_urls.iter().map(|url| url.as_str()))
            .map(Url::parse)
            .collect()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub name: Option<String>,
    pub url: String,
    // pub explicit: Option<bool>,
    pub default: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthSource {
    EnvVar,
    Keyring,
}

#[cfg(test)]
mod tests {
    use url::Url;

    use crate::{
        config::{Config, ConfigProject, Index},
        lock::Source,
    };

    #[test]
    fn default_config() {
        let config = Config::default();

        assert_eq!(config.quiet, None);
        assert_eq!(config.verbose, None);
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
            quiet: Some(true),
            verbose: Some(false),
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
}
