// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use thiserror::Error;

use super::Config;

pub const CONFIG_DIR: &str = "sysand";
pub const CONFIG_FILE: &str = "sysand.toml";

#[derive(Error, Debug)]
pub enum ConfigReadError {
    #[error("toml deserialization error")]
    TomlError(#[from] toml::de::Error),
    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),
}

pub fn get_config<P: AsRef<Path>>(path: P) -> Result<Config, ConfigReadError> {
    if path.as_ref().is_file() {
        let contents = std::fs::read_to_string(path.as_ref())?;
        Ok(toml::from_str(&contents)?)
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

    use tempfile::tempdir;

    use crate::config::{Config, Index, local_fs};

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
