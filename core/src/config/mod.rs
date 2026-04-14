// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{Deserialize, Serialize};
use url::Url;

use crate::lock::Source;

#[cfg(feature = "filesystem")]
pub mod local_fs;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
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
            mut indexes,
            mut projects,
        } = config;
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
#[path = "./mod_tests.rs"]
mod tests;
