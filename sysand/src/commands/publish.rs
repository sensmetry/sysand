// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use camino::Utf8PathBuf;
use sysand_core::{
    auth::StandardHTTPAuthentication, build::default_kpar_file_name,
    commands::publish::do_publish_kpar, config::Config, context::ProjectContext,
};
use url::Url;

use crate::{CliError, DEFAULT_INDEX_URL};

fn resolve_publish_kpar_path(
    path: Option<Utf8PathBuf>,
    ctx: &ProjectContext,
) -> Result<Utf8PathBuf> {
    Ok(if let Some(path) = path {
        path
    } else {
        let current_project = ctx
            .current_project
            .as_ref()
            .ok_or(CliError::MissingProjectCurrentDir)?;
        let mut output_dir = ctx
            .current_workspace
            .as_ref()
            .map(|workspace| workspace.root_path())
            .unwrap_or(&current_project.project_path)
            .join("output");
        let name = default_kpar_file_name(current_project)?;
        output_dir.push(name);
        output_dir
    })
}

fn resolve_publish_index_url(index: Option<Url>, config: &Config) -> Result<Url> {
    if let Some(index_url) = index {
        return Ok(index_url);
    }

    if let Some(index_url) = config
        .indexes
        .iter()
        .find(|index| index.default.unwrap_or(false))
        .map(|index| index.url.as_str())
        .or_else(|| config.indexes.first().map(|index| index.url.as_str()))
    {
        Url::parse(index_url).map_err(|e| anyhow!("invalid index URL in configuration: {e}"))
    } else {
        Ok(Url::parse(DEFAULT_INDEX_URL).expect("default publish index URL must be valid"))
    }
}

pub fn command_publish(
    path: Option<Utf8PathBuf>,
    index: Option<Url>,
    ctx: &ProjectContext,
    config: &Config,
    auth_policy: Arc<StandardHTTPAuthentication>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let kpar_path = resolve_publish_kpar_path(path, ctx)?;
    if !kpar_path.is_file() {
        bail!("kpar file not found at `{kpar_path}`, run `sysand build` first");
    }
    let index_url = resolve_publish_index_url(index, config)?;
    let publish_auth_policy =
        Arc::new(Arc::unwrap_or_clone(auth_policy).into_publish_authentication()?);
    let response = do_publish_kpar(kpar_path, index_url, publish_auth_policy, client, runtime)?;

    let header = sysand_core::style::get_style_config().header;
    let published = "Published";
    if response.is_new_project {
        log::info!("{header}{published:>12}{header:#} new project successfully");
    } else {
        log::info!("{header}{published:>12}{header:#} new release successfully");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_publish_index_url;
    use crate::DEFAULT_INDEX_URL;
    use sysand_core::config::{Config, Index};
    use url::Url;

    #[test]
    fn resolve_publish_index_url_prefers_explicit_flag() {
        let config = Config {
            indexes: vec![Index {
                url: "https://config.example.com".to_string(),
                default: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        };

        let url = resolve_publish_index_url(
            Some(Url::parse("https://cli.example.com").unwrap()),
            &config,
        )
        .unwrap();

        assert_eq!(url.as_str(), "https://cli.example.com/");
    }

    #[test]
    fn resolve_publish_index_url_prefers_config_default() {
        let config = Config {
            indexes: vec![
                Index {
                    url: "https://non-default.example.com".to_string(),
                    default: Some(false),
                    ..Default::default()
                },
                Index {
                    url: "https://default.example.com".to_string(),
                    default: Some(true),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let url = resolve_publish_index_url(None, &config).unwrap();

        assert_eq!(url.as_str(), "https://default.example.com/");
    }

    #[test]
    fn resolve_publish_index_url_falls_back_to_first_non_default() {
        let config = Config {
            indexes: vec![
                Index {
                    url: "https://first.example.com".to_string(),
                    ..Default::default()
                },
                Index {
                    url: "https://second.example.com".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let url = resolve_publish_index_url(None, &config).unwrap();

        assert_eq!(url.as_str(), "https://first.example.com/");
    }

    #[test]
    fn resolve_publish_index_url_falls_back_to_builtin_default() {
        let config = Config::default();
        let url = resolve_publish_index_url(None, &config).unwrap();

        assert_eq!(url.as_str(), format!("{DEFAULT_INDEX_URL}/"));
    }

    #[test]
    fn resolve_publish_index_url_reports_invalid_config_url() {
        let config = Config {
            indexes: vec![Index {
                url: "not-a-url".to_string(),
                default: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        };

        let error = resolve_publish_index_url(None, &config)
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid index URL in configuration"));
    }
}
