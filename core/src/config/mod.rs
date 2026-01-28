// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{Deserialize, Serialize};
use url::Url;

#[cfg(feature = "filesystem")]
pub mod local_fs;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub quiet: Option<bool>,
    pub verbose: Option<bool>,
    pub index: Option<Vec<Index>>,
    pub auth: Option<Vec<AuthSource>>,
}

impl Config {
    pub fn merge(&mut self, config: Config) {
        self.quiet = self.quiet.or(config.quiet);
        self.verbose = self.verbose.or(config.verbose);
        extend_option_vec(&mut self.index, config.index);

        if let Some(auth) = config.auth {
            self.auth = Some(auth.clone());
        }
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
        let mut indexes: Vec<_> = self.index.iter().flat_map(|v| v.iter()).collect();

        indexes.sort_by_key(|i| i.default.unwrap_or(false));

        let has_default = indexes
            .last()
            .and_then(|index| index.default)
            .unwrap_or(false);

        let end: Vec<String> = if has_default {
            std::iter::empty::<String>().collect()
        } else {
            default_urls
        };

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
                self.index
                    .iter()
                    .flat_map(|v| v.iter())
                    .filter(|i| !i.default.unwrap_or(false))
                    .map(|i| i.url.as_str()),
            )
            .chain(default_urls.iter().map(|url| url.as_str()))
            .map(Url::parse)
            .collect()
    }
}

fn extend_option_vec<T>(target: &mut Option<Vec<T>>, src: Option<Vec<T>>) {
    if let Some(mut src_vec) = src {
        target.get_or_insert_with(Vec::new).append(&mut src_vec);
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

    use crate::config::{Config, Index};

    #[test]
    fn default_config() {
        let config = Config::default();

        assert_eq!(config.quiet, None);
        assert_eq!(config.verbose, None);
        assert_eq!(config.index, None);
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
            index: Some(vec![Index {
                url: "http://www.example.com".to_string(),
                ..Default::default()
            }]),
            auth: None,
        };
        defaults.merge(config.clone());

        assert_eq!(defaults, config);
    }

    #[test]
    fn index_urls_without_default() {
        let config = Config {
            index: Some(vec![Index {
                url: "http://www.index.com".to_string(),
                ..Default::default()
            }]),
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
            index: Some(vec![
                Index {
                    url: "http://www.config-default.com".to_string(),
                    default: Some(true),
                    ..Default::default()
                },
                Index {
                    url: "http://www.index.com".to_string(),
                    ..Default::default()
                },
            ]),
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
            index: Some(vec![
                Index {
                    url: "http://www.config-default.com".to_string(),
                    default: Some(true),
                    ..Default::default()
                },
                Index {
                    url: "http://www.index.com".to_string(),
                    ..Default::default()
                },
            ]),
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
