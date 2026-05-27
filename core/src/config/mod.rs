// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use serde::{Deserialize, Serialize};
use toml_edit::{InlineTable, Value};
use typed_path::Utf8UnixPathBuf;
use url::Url;

use crate::project::utils::{deserialize_unix_path, serialize_unix_path};

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
    pub sources: Vec<OverrideSource>,
}

#[derive(Clone, Eq, Debug, Deserialize, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(untagged)]
pub enum OverrideSource {
    // Path must be a Unix path relative to workspace root
    Editable {
        #[serde(
            deserialize_with = "deserialize_unix_path",
            serialize_with = "serialize_unix_path"
        )]
        editable: Utf8UnixPathBuf,
    },
    LocalSrc {
        #[serde(
            deserialize_with = "deserialize_unix_path",
            serialize_with = "serialize_unix_path"
        )]
        src_path: Utf8UnixPathBuf,
    },
    LocalKpar {
        #[serde(
            deserialize_with = "deserialize_unix_path",
            serialize_with = "serialize_unix_path"
        )]
        kpar_path: Utf8UnixPathBuf,
    },
    RemoteKpar {
        remote_kpar: String,
    },
    // TODO: it doesn't make sense to have this in url shape; it should be a
    // publisher/name/IRI
    // IndexKpar {
    //     index_kpar: String,
    // },
    RemoteSrc {
        remote_src: String,
    },
    RemoteGit {
        remote_git: String,
    },
}

impl OverrideSource {
    pub fn to_toml(&self) -> InlineTable {
        let mut table = InlineTable::new();
        match self {
            Self::Editable { editable } => {
                debug_assert!(
                    editable.is_relative(),
                    "editable project path is absolute: `{editable}`"
                );
                table.insert("editable", Value::from(editable.as_str()));
            }
            Self::LocalKpar { kpar_path } => {
                table.insert("kpar_path", Value::from(kpar_path.as_str()));
            }
            Self::LocalSrc { src_path } => {
                table.insert("src_path", Value::from(src_path.as_str()));
            }
            Self::RemoteGit { remote_git } => {
                table.insert("remote_git", Value::from(remote_git));
            }
            Self::RemoteKpar { remote_kpar } => {
                table.insert("remote_kpar", Value::from(remote_kpar));
            }
            Self::RemoteSrc { remote_src } => {
                table.insert("remote_src", Value::from(remote_src));
            }
        }
        table
    }
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
