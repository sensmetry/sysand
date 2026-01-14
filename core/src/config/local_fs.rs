// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fs, path::Path};

use camino::Utf8Path;
use thiserror::Error;

use super::Config;
use crate::project::utils::FsIoError;

pub const CONFIG_DIR: &str = "sysand";
pub const CONFIG_FILE: &str = "sysand.toml";

#[derive(Error, Debug)]
pub enum ConfigReadError {
    #[error("failed to deserialize TOML file `{0}`: {1}")]
    Toml(Box<Utf8Path>, toml::de::Error),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl From<FsIoError> for ConfigReadError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

pub fn get_config<P: AsRef<Path>>(path: P) -> Result<Config, ConfigReadError> {
    if path.as_ref().is_file() {
        let contents = {
            fs::read_to_string(path.as_ref()).map_err(|e| {
                Box::new(FsIoError::ReadFile(
                    path.as_ref().to_string_lossy().into_owned().into(),
                    e,
                ))
            })
        }?;
        Ok(toml::from_str(&contents).map_err(|e| {
            ConfigReadError::Toml(
                path.as_ref().to_string_lossy().into_owned().as_str().into(),
                e,
            )
        })?)
    } else {
        Ok(Config::default())
    }
}

pub fn load_configs<P: AsRef<Utf8Path>>(working_dir: P) -> Result<Config, ConfigReadError> {
    let mut config = dirs::config_dir().map_or_else(
        || Ok(Config::default()),
        |mut path| {
            path.push(CONFIG_DIR);
            path.push(CONFIG_FILE);
            get_config(path)
        },
    )?;
    config.merge(get_config(working_dir.as_ref().join(CONFIG_FILE))?);

    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use crate::config::{Config, Index, local_fs};
    use camino_tempfile::tempdir;

    #[test]
    fn load_configs() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join(local_fs::CONFIG_FILE);
        let mut config_file = std::fs::File::create(config_path).unwrap();
        let config = Config {
            quiet: Some(true),
            verbose: Some(false),
            index: Some(vec![Index {
                url: "http://www.example.com".to_string(),
                ..Default::default()
            }]),
        };
        config_file
            .write_all(toml::to_string_pretty(&config).unwrap().as_bytes())
            .unwrap();

        let config_read = local_fs::load_configs(dir.path()).unwrap();

        assert_eq!(config_read, config);
    }
}
