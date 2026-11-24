// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{io::ErrorKind, str::FromStr as _};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use thiserror::Error;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

use super::Config;
use crate::{
    config::OverrideSource,
    paths::{PathKind, UnixPathError, parse_unix_path},
    project::utils::{FsIoError, wrapfs},
    utils::multiline_array,
};

pub const CONFIG_DIR: &str = "sysand";
pub const CONFIG_FILE: &str = "sysand.toml";

#[derive(Error, Debug)]
pub enum ConfigReadError {
    #[error("failed to deserialize TOML file `{0}`")]
    Toml(Box<Utf8Path>, #[source] toml::de::Error),
    #[error("invalid project source path in configuration file `{0}`")]
    SourcePath(Box<Utf8Path>, #[source] UnixPathError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl From<FsIoError> for ConfigReadError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

/// Read the configuration file at `path`, or the default configuration if
/// there is no file there
pub fn get_config<P: AsRef<Utf8Path>>(path: P) -> Result<Config, ConfigReadError> {
    if wrapfs::is_file(path.as_ref())? {
        read_config(path)
    } else {
        Ok(Config::default())
    }
}

/// Read the configuration file at `path`, which must exist
pub fn read_config<P: AsRef<Utf8Path>>(path: P) -> Result<Config, ConfigReadError> {
    let path = path.as_ref();
    let contents = wrapfs::read_to_string(path)?;
    parse_config(path, &contents)
}

pub fn load_configs<P: AsRef<Utf8Path>>(working_dir: P) -> Result<Config, ConfigReadError> {
    let user_config = dirs::config_dir().map(|mut path| {
        path.push(CONFIG_DIR);
        path.push(CONFIG_FILE);
        Utf8PathBuf::from_path_buf(path).unwrap()
    });
    load_configs_from(user_config.as_deref(), working_dir)
}

/// [`load_configs`] with the user configuration file location injected
pub(crate) fn load_configs_from<P: AsRef<Utf8Path>>(
    user_config: Option<&Utf8Path>,
    working_dir: P,
) -> Result<Config, ConfigReadError> {
    let mut config = match user_config {
        Some(path) => get_config(path)?,
        None => Config::default(),
    };
    config.merge(get_config(working_dir.as_ref().join(CONFIG_FILE))?);

    Ok(config)
}

#[derive(Error, Debug)]
pub enum ConfigProjectSourceError {
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("`{0}` is not a file")]
    NotAFile(String),
    #[error("failed to parse configuration file at `{0}`")]
    TomlEdit(Utf8PathBuf, #[source] toml_edit::TomlError),
    #[error("{0}")]
    InvalidProjects(String),
    #[error(transparent)]
    Invalid(#[from] ConfigReadError),
}

/// Refuse configuration text that [`read_config`] would refuse.
fn validate_config_text(path: &Utf8Path, text: &str) -> Result<(), ConfigProjectSourceError> {
    parse_config(path, text)?;
    Ok(())
}

/// The configuration `text` of the file at `path`. A local source path
/// takes the rule of a usage path.
fn parse_config(path: &Utf8Path, text: &str) -> Result<Config, ConfigReadError> {
    let config: Config = toml::from_str(text).map_err(|e| ConfigReadError::Toml(path.into(), e))?;
    for source in config.projects.iter().flat_map(|project| &project.sources) {
        let (local, kind) = match source {
            OverrideSource::Editable { editable: local }
            | OverrideSource::LocalSrc { src_path: local } => (local, PathKind::Directory),
            OverrideSource::LocalKpar { kpar_path: local } => (local, PathKind::File),
            OverrideSource::RemoteKpar { remote_kpar: _ }
            | OverrideSource::RemoteSrc { remote_src: _ }
            | OverrideSource::RemoteGit { remote_git: _ } => continue,
        };
        parse_unix_path(local.as_str(), kind)
            .map_err(|e| ConfigReadError::SourcePath(path.into(), e))?;
    }
    Ok(config)
}

pub fn add_project_source_to_config<P: AsRef<Utf8Path>>(
    config_path: P,
    iri: Iri<&str>,
    source: &OverrideSource,
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
            log::info!("{header}{creating:>12}{header:#} configuration file at `{config_path}`");
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
        .ok_or_else(|| {
            ConfigProjectSourceError::InvalidProjects(
                "`project` should always be an array of tables".to_owned(),
            )
        })?;

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

    let edited = config.to_string();
    validate_config_text(config_path, &edited)?;
    wrapfs::write(config_path, edited)?;

    Ok(())
}

pub fn remove_project_source_from_config<P: AsRef<Utf8Path>>(
    config_path: P,
    iri: Iri<&str>,
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
    validate_config_text(config_path, &contents)?;
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
                "{header}{removing:>12}{header:#} empty configuration file at `{config_path}`",
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
#[path = "./local_fs_tests.rs"]
mod tests;
