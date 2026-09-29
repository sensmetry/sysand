// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use serde::{Deserialize, Serialize};
use toml_edit::{InlineTable, Value};
use typed_path::Utf8UnixPathBuf;

use crate::index_location::IndexLocation;
use crate::project::utils::{deserialize_unix_path, serialize_unix_path};

#[cfg(feature = "filesystem")]
pub mod local_fs;

// TODO: validate IRIs and paths
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "index", skip_serializing_if = "Vec::is_empty", default)]
    pub indexes: Vec<Index>,
    #[serde(rename = "project", skip_serializing_if = "Vec::is_empty", default)]
    pub projects: Vec<ConfigProject>,
    // pub auth: Option<Vec<AuthSource>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProject {
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub identifiers: Vec<Iri<String>>,
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
    // Path must be a Unix path relative to workspace root
    LocalSrc {
        #[serde(
            deserialize_with = "deserialize_unix_path",
            serialize_with = "serialize_unix_path"
        )]
        src_path: Utf8UnixPathBuf,
    },
    // Path must be a Unix path relative to workspace root
    LocalKpar {
        #[serde(
            deserialize_with = "deserialize_unix_path",
            serialize_with = "serialize_unix_path"
        )]
        kpar_path: Utf8UnixPathBuf,
    },
    RemoteKpar {
        remote_kpar: Iri<String>,
    },
    // TODO: it doesn't make sense to have this in url shape; it should be a
    // publisher/name/IRI
    // IndexKpar {
    //     index_kpar: String,
    // },
    RemoteSrc {
        remote_src: Iri<String>,
    },
    RemoteGit {
        remote_git: Iri<String>,
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
                table.insert("remote_git", Value::from(remote_git.as_str()));
            }
            Self::RemoteKpar { remote_kpar } => {
                table.insert("remote_kpar", Value::from(remote_kpar.as_str()));
            }
            Self::RemoteSrc { remote_src } => {
                table.insert("remote_src", Value::from(remote_src.as_str()));
            }
        }
        table
    }
}

impl Config {
    pub fn merge(&mut self, config: Self) {
        let Self {
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
        index_urls: Vec<IndexLocation>,
        default_urls: Vec<IndexLocation>,
        default_override_urls: Vec<IndexLocation>,
    ) -> Vec<IndexLocation> {
        if default_override_urls.is_empty() {
            self.index_urls_no_default_override(index_urls, default_urls)
        } else {
            self.index_urls_with_default_override(index_urls, default_override_urls)
        }
    }

    /// Locations of the indexes marked `default = true`, in configuration
    /// order
    pub fn default_index_locations(&self) -> Vec<IndexLocation> {
        self.indexes
            .iter()
            .filter(|i| i.default.unwrap_or(false))
            .map(|i| i.url.clone())
            .collect()
    }

    fn index_urls_no_default_override(
        &self,
        index_urls: Vec<IndexLocation>,
        default_urls: Vec<IndexLocation>,
    ) -> Vec<IndexLocation> {
        let mut indexes = self.indexes.clone();

        indexes.sort_by_key(|i| i.default.unwrap_or(false));

        let has_default = indexes
            .last()
            .and_then(|index| index.default)
            .unwrap_or(false);

        let end = if has_default { vec![] } else { default_urls };

        index_urls
            .into_iter()
            .chain(indexes.into_iter().map(|i| i.url))
            .chain(end)
            .collect()
    }

    fn index_urls_with_default_override(
        &self,
        index_urls: Vec<IndexLocation>,
        default_urls: Vec<IndexLocation>,
    ) -> Vec<IndexLocation> {
        index_urls
            .into_iter()
            .chain(
                self.indexes
                    .iter()
                    .filter(|i| !i.default.unwrap_or(false))
                    .map(|i| i.url.clone()),
            )
            .chain(default_urls)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    name: Option<String>,
    url: IndexLocation,
    // explicit: Option<bool>,
    default: Option<bool>,
}

impl Index {
    pub fn new_url(url: IndexLocation) -> Self {
        Self {
            name: None,
            url,
            default: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthSource {
    EnvVar,
    Keyring,
}

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
