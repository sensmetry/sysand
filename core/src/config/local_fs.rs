// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fs, io::ErrorKind, str::FromStr};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

use super::Config;
use crate::{
    lock::{Source, multiline_array},
    project::utils::{FsIoError, wrapfs},
};

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

pub fn get_config<P: AsRef<Utf8Path>>(path: P) -> Result<Config, ConfigReadError> {
    if wrapfs::is_file(path.as_ref())? {
        let contents = {
            fs::read_to_string(path.as_ref())
                .map_err(|e| Box::new(FsIoError::ReadFile(path.as_ref().to_owned(), e)))
        }?;
        Ok(toml::from_str(&contents)
            .map_err(|e| ConfigReadError::Toml(path.as_ref().to_owned().into(), e))?)
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
            get_config(Utf8PathBuf::from_path_buf(path).unwrap())
        },
    )?;
    config.merge(get_config(working_dir.as_ref().join(CONFIG_FILE))?);

    Ok(config)
}

#[derive(Error, Debug)]
pub enum ConfigProjectSourceError {
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("`{0}` is not a file")]
    NotAFile(String),
    #[error("failed to parse configuration file at `{0}`:\n{1}")]
    TomlEdit(Utf8PathBuf, toml_edit::TomlError),
    #[error("{0}")]
    InvalidProjects(String),
}

pub fn add_project_source_to_config<P: AsRef<Utf8Path>, S: AsRef<str>>(
    config_path: P,
    iri: S,
    source: &Source,
) -> Result<(), ConfigProjectSourceError> {
    let config_path = config_path.as_ref();
    let sources = multiline_array(std::iter::once(source.to_toml()));
    let contents = match wrapfs::metadata(config_path) {
        Ok(metadata) if metadata.is_file() => wrapfs::read_to_string(config_path)?,
        Ok(_) => {
            return Err(ConfigProjectSourceError::NotAFile(config_path.to_string()));
        }
        Err(err) if matches!(err.as_ref(), FsIoError::Metadata(_, e) if e.kind() == ErrorKind::NotFound) =>
        {
            let creating = "Creating";
            let header = crate::style::get_style_config().header;
            log::info!(
                "{header}{creating:>12}{header:#} configuration file at `{}`",
                config_path,
            );
            String::new()
        }
        Err(err) => return Err(ConfigProjectSourceError::Io(err)),
    };
    let mut config = DocumentMut::from_str(&contents)
        .map_err(|err| ConfigProjectSourceError::TomlEdit(config_path.to_owned(), err))?;
    let projects = config
        .as_table_mut()
        .entry("project")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or(ConfigProjectSourceError::InvalidProjects(
            "`project` should always be an array of tables".to_string(),
        ))?;

    if let Some(project) = projects.iter_mut().find(|table| {
        table
            .get("identifiers")
            .and_then(|n| n.as_array())
            .is_some_and(|arr| {
                arr.iter()
                    .any(|identifier| identifier.as_str() == Some(iri.as_ref()))
            })
    }) {
        project["sources"] = toml_edit::value(sources);
    } else {
        let mut project = Table::new();
        project["identifiers"] =
            toml_edit::value(multiline_array(std::iter::once(Value::from(iri.as_ref()))));
        project["sources"] = toml_edit::value(sources);

        projects.push(project);
    }

    let adding = "Adding";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{adding:>12}{header:#} source for `{}` to configuration file at `{}`",
        iri.as_ref(),
        config_path,
    );

    wrapfs::write(config_path, config.to_string())?;

    Ok(())
}

pub fn remove_project_source_from_config<P: AsRef<Utf8Path>, S: AsRef<str>>(
    config_path: P,
    iri: S,
) -> Result<bool, ConfigProjectSourceError> {
    let config_path = config_path.as_ref();
    let contents = match wrapfs::metadata(config_path) {
        Ok(metadata) if metadata.is_file() => wrapfs::read_to_string(config_path)?,
        Ok(_) => {
            return Err(ConfigProjectSourceError::NotAFile(config_path.to_string()));
        }
        Err(err) if matches!(err.as_ref(), FsIoError::Metadata(_, e) if e.kind() == ErrorKind::NotFound) =>
        {
            return Ok(false);
        }
        Err(err) => return Err(ConfigProjectSourceError::Io(err)),
    };
    let mut config = DocumentMut::from_str(&contents)
        .map_err(|err| ConfigProjectSourceError::TomlEdit(config_path.to_owned(), err))?;
    let Some(projects) = config
        .as_table_mut()
        .get_mut("project")
        .and_then(Item::as_array_of_tables_mut)
    else {
        return Ok(false);
    };

    let remove_index = projects.iter().position(|project| {
        project
            .get("identifiers")
            .and_then(|n| n.as_array())
            .is_some_and(|arr| {
                arr.iter()
                    .any(|identifier| identifier.as_str() == Some(iri.as_ref()))
            })
    });

    if let Some(index) = remove_index {
        let removing = "Removing";
        let header = crate::style::get_style_config().header;
        log::info!(
            "{header}{removing:>12}{header:#} source for `{}` from configuration file at `{}`",
            iri.as_ref(),
            config_path,
        );

        projects.remove(index);
        let contents = config.to_string();

        if contents.is_empty() {
            let removing = "Removing";
            log::info!(
                "{header}{removing:>12}{header:#} empty configuration file at `{}`",
                config_path,
            );
            wrapfs::remove_file(config_path)?;
        } else {
            wrapfs::write(config_path, contents)?;
        }

        return Ok(true);
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io::Write};

    use camino_tempfile::tempdir;

    use crate::{
        config::{Config, ConfigProject, Index, local_fs},
        lock::Source,
        project::utils::wrapfs,
    };

    #[test]
    fn load_configs() -> Result<(), Box<dyn Error>> {
        let dir = tempdir()?;
        let config_path = dir.path().join(local_fs::CONFIG_FILE);
        let mut config_file = wrapfs::File::create(config_path)?;
        let config = Config {
            indexes: vec![Index {
                url: "http://www.example.com".to_string(),
                ..Default::default()
            }],
            projects: vec![],
            // auth: None,
        };
        config_file.write_all(toml::to_string_pretty(&config)?.as_bytes())?;

        let config_read = local_fs::load_configs(dir.path())?;

        assert_eq!(config_read, config);

        Ok(())
    }

    #[test]
    fn add_project_source_to_config() -> Result<(), Box<dyn Error>> {
        let dir = tempdir()?;
        let config_path = dir.path().join(local_fs::CONFIG_FILE);
        let iri = "urn:kpar:test";
        let source = Source::LocalSrc {
            src_path: "local/test".into(),
        };

        local_fs::add_project_source_to_config(&config_path, iri, &source)?;

        let config = Config {
            indexes: vec![],
            projects: vec![ConfigProject {
                identifiers: vec![iri.to_string()],
                sources: vec![source],
            }],
        };

        assert_eq!(
            config,
            toml::from_str(wrapfs::read_to_string(config_path)?.as_str())?,
        );

        Ok(())
    }

    #[test]
    fn remove_project_source_from_config() -> Result<(), Box<dyn Error>> {
        let dir = tempdir()?;
        let config_path = dir.path().join(local_fs::CONFIG_FILE);
        let mut config_file = wrapfs::File::create(&config_path)?;
        let iri = "urn:kpar:test";
        let source = Source::LocalSrc {
            src_path: "local/test".into(),
        };
        let config = Config {
            indexes: vec![],
            projects: vec![ConfigProject {
                identifiers: vec![iri.to_string()],
                sources: vec![source],
            }],
        };
        config_file.write_all(toml::to_string_pretty(&config)?.as_bytes())?;

        local_fs::remove_project_source_from_config(&config_path, iri)?;

        assert!(!config_path.is_file());

        Ok(())
    }
}
