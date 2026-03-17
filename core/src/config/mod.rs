// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Component;

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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub build: Option<BuildConfig>,
    // pub auth: Option<Vec<AuthSource>>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Override the README source path (relative to project root).
    /// Default: `"README.md"`. Set to `false` to disable README bundling.
    /// Must have a `.md` extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme: Option<ReadmeConfig>,
}

/// Resolved README bundling instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedReadme<'a> {
    /// No README should be bundled.
    Disabled,
    /// Bundle from this path; if the file is missing, silently skip (implicit default).
    DefaultIfExists(&'a str),
    /// Bundle from this path; if the file is missing, error (explicitly configured).
    Required(&'a str),
}

impl BuildConfig {
    /// Resolve and validate the README path from config.
    ///
    /// - No `readme` field → [`ResolvedReadme::DefaultIfExists("README.md")`]
    /// - `readme = true` → [`ResolvedReadme::Required("README.md")`]
    /// - `readme = false` → [`ResolvedReadme::Disabled`]
    /// - `readme = "docs/CUSTOM.md"` → [`ResolvedReadme::Required`] with validated path
    ///
    /// Validates that the path is relative, uses forward slashes, does not escape
    /// the project root via `..` traversal, and has a `.md` extension.
    pub fn resolve_readme_path(&self) -> Result<ResolvedReadme<'_>, ReadmeConfigError> {
        let (path, explicit) = match &self.readme {
            None => return Ok(ResolvedReadme::DefaultIfExists("README.md")),
            Some(ReadmeConfig::Disabled) => return Ok(ResolvedReadme::Disabled),
            Some(ReadmeConfig::Enabled) => ("README.md", true),
            Some(ReadmeConfig::Path(path)) => (path.as_str(), true),
        };

        if path.contains('\\') {
            return Err(ReadmeConfigError::BackSlashes(path.to_owned()));
        }

        if std::path::Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(ReadmeConfigError::NotRelative(path.to_owned()));
        }

        if !path.ends_with(".md") {
            return Err(ReadmeConfigError::NotMarkdown(path.to_owned()));
        }

        if explicit {
            Ok(ResolvedReadme::Required(path))
        } else {
            Ok(ResolvedReadme::DefaultIfExists(path))
        }
    }
}

/// Configuration value for the `readme` field in `[build]`.
///
/// Accepts a string path, `true` (explicit default), or `false` to disable README bundling.
#[derive(Debug, Clone, PartialEq)]
pub enum ReadmeConfig {
    /// Disable README bundling (`readme = false`).
    Disabled,
    /// Explicitly enable README bundling with default `README.md` (`readme = true`).
    Enabled,
    /// Path to the README file, relative to the project root.
    Path(String),
}

impl Serialize for ReadmeConfig {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ReadmeConfig::Disabled => serializer.serialize_bool(false),
            ReadmeConfig::Enabled => serializer.serialize_bool(true),
            ReadmeConfig::Path(path) => serializer.serialize_str(path),
        }
    }
}

impl<'de> Deserialize<'de> for ReadmeConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ReadmeConfigVisitor;

        impl<'de> serde::de::Visitor<'de> for ReadmeConfigVisitor {
            type Value = ReadmeConfig;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string path or `false`")
            }

            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                if v {
                    Ok(ReadmeConfig::Enabled)
                } else {
                    Ok(ReadmeConfig::Disabled)
                }
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ReadmeConfig::Path(v.to_owned()))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(ReadmeConfig::Path(v))
            }
        }

        deserializer.deserialize_any(ReadmeConfigVisitor)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReadmeConfigError {
    #[error("README path `{0}` must use forward slashes")]
    BackSlashes(String),
    #[error("README path `{0}` must be a clean relative path (no `..`, `.`, or absolute prefixes)")]
    NotRelative(String),
    #[error("README path `{0}` must have a `.md` extension")]
    NotMarkdown(String),
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
            build,
        } = config;
        self.indexes.append(&mut indexes);
        self.projects.append(&mut projects);
        if build.is_some() {
            self.build = build;
        }

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
        config::{
            BuildConfig, Config, ConfigProject, Index, ReadmeConfig, ReadmeConfigError,
            ResolvedReadme,
        },
        lock::Source,
    };

    #[test]
    fn readme_default_when_omitted() {
        let config = BuildConfig::default();
        assert_eq!(
            config.resolve_readme_path().unwrap(),
            ResolvedReadme::DefaultIfExists("README.md")
        );
    }

    #[test]
    fn readme_disabled() {
        let config = BuildConfig {
            readme: Some(ReadmeConfig::Disabled),
        };
        assert_eq!(
            config.resolve_readme_path().unwrap(),
            ResolvedReadme::Disabled
        );
    }

    #[test]
    fn readme_true_is_required() {
        let config = BuildConfig {
            readme: Some(ReadmeConfig::Enabled),
        };
        assert_eq!(
            config.resolve_readme_path().unwrap(),
            ResolvedReadme::Required("README.md")
        );
    }

    #[test]
    fn readme_custom_path() {
        let config = BuildConfig {
            readme: Some(ReadmeConfig::Path("docs/README.md".into())),
        };
        assert_eq!(
            config.resolve_readme_path().unwrap(),
            ResolvedReadme::Required("docs/README.md")
        );
    }

    #[test]
    fn readme_rejects_backslashes() {
        let config = BuildConfig {
            readme: Some(ReadmeConfig::Path("docs\\README.md".into())),
        };
        assert!(matches!(
            config.resolve_readme_path(),
            Err(ReadmeConfigError::BackSlashes(_))
        ));
    }

    #[test]
    fn readme_rejects_parent_traversal() {
        let config = BuildConfig {
            readme: Some(ReadmeConfig::Path("../escape.md".into())),
        };
        assert!(matches!(
            config.resolve_readme_path(),
            Err(ReadmeConfigError::NotRelative(_))
        ));
    }

    #[test]
    fn readme_rejects_absolute_path() {
        let config = BuildConfig {
            readme: Some(ReadmeConfig::Path("/absolute.md".into())),
        };
        assert!(matches!(
            config.resolve_readme_path(),
            Err(ReadmeConfigError::NotRelative(_))
        ));
    }

    #[test]
    fn readme_rejects_non_markdown() {
        let config = BuildConfig {
            readme: Some(ReadmeConfig::Path("README.txt".into())),
        };
        assert!(matches!(
            config.resolve_readme_path(),
            Err(ReadmeConfigError::NotMarkdown(_))
        ));
    }

    #[test]
    fn readme_rejects_dot_prefix() {
        let config = BuildConfig {
            readme: Some(ReadmeConfig::Path("./README.md".into())),
        };
        assert!(matches!(
            config.resolve_readme_path(),
            Err(ReadmeConfigError::NotRelative(_))
        ));
    }

    #[test]
    fn readme_serde_roundtrip_false() {
        let config: BuildConfig = toml::from_str("readme = false").unwrap();
        assert_eq!(config.readme, Some(ReadmeConfig::Disabled));
    }

    #[test]
    fn readme_serde_roundtrip_true() {
        let config: BuildConfig = toml::from_str("readme = true").unwrap();
        assert_eq!(config.readme, Some(ReadmeConfig::Enabled));
    }

    #[test]
    fn readme_serde_roundtrip_path() {
        let config: BuildConfig = toml::from_str("readme = \"custom.md\"").unwrap();
        assert_eq!(config.readme, Some(ReadmeConfig::Path("custom.md".into())));
    }

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
            ..Default::default()
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
