// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use thiserror::Error;

use super::Config;
use crate::project::utils::{FsIoError, wrapfs};

pub const CONFIG_DIR: &str = "sysand";
pub const CONFIG_FILE: &str = "sysand.toml";

#[derive(Error, Debug)]
pub enum ConfigReadError {
    #[error("failed to deserialize TOML file `{0}`: {1}")]
    Toml(Box<Path>, toml::de::Error),
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
        let contents = wrapfs::read_to_string(&path)?;
        Ok(
            toml::from_str(&contents)
                .map_err(|e| ConfigReadError::Toml(path.as_ref().into(), e))?,
        )
    } else {
        Ok(Config::default())
    }
}

pub fn load_configs<P: AsRef<Path>>(working_dir: P) -> Result<Config, ConfigReadError> {
    let mut config = dirs::config_dir().map_or_else(
        || Ok(Config::default()),
        |path| get_config(path.join(CONFIG_DIR).join(CONFIG_FILE)),
    )?;
    config.merge(get_config(working_dir.as_ref().join(CONFIG_FILE))?);

    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use crate::config::{Config, Index, local_fs};
    use tempfile::tempdir;

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
