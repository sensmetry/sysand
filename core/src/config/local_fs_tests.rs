// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{error::Error, io::Write as _};

use camino_tempfile::tempdir;

use crate::{
    config::{Config, ConfigProject, Index, OverrideSource, local_fs},
    project::utils::wrapfs,
};

#[test]
fn load_configs() -> Result<(), Box<dyn Error>> {
    let dir = tempdir()?;
    let config_path = dir.path().join(local_fs::CONFIG_FILE);
    let mut config_file = wrapfs::File::create(config_path)?;
    let config = Config {
        indexes: vec![Index {
            url: "http://www.example.com".to_owned(),
            ..Default::default()
        }],
        projects: vec![],
        // auth: None,
    };
    config_file.write_all(toml::to_string_pretty(&config)?.as_bytes())?;

    // `None` user config: `load_configs` itself would read the
    // developer's real `~/.config/sysand/sysand.toml` and make the
    // assertion environment-dependent.
    let config_read = local_fs::load_configs_from(None, dir.path())?;

    assert_eq!(config_read, config);

    Ok(())
}

#[test]
fn load_configs_merges_user_config_before_working_dir() -> Result<(), Box<dyn Error>> {
    let user_dir = tempdir()?;
    let user_path = user_dir.path().join(local_fs::CONFIG_FILE);
    let user_config = Config {
        indexes: vec![Index {
            url: "http://user.example.com".to_owned(),
            ..Default::default()
        }],
        projects: vec![],
    };
    wrapfs::write(&user_path, toml::to_string(&user_config)?)?;

    let working_dir = tempdir()?;
    let working_config = Config {
        indexes: vec![Index {
            url: "http://working.example.com".to_owned(),
            ..Default::default()
        }],
        projects: vec![],
    };
    wrapfs::write(
        working_dir.path().join(local_fs::CONFIG_FILE),
        toml::to_string(&working_config)?,
    )?;

    let config_read = local_fs::load_configs_from(Some(&user_path), working_dir.path())?;

    let mut expected = user_config;
    expected.merge(working_config);
    assert_eq!(config_read, expected);

    Ok(())
}

#[test]
fn add_project_source_to_config() -> Result<(), Box<dyn Error>> {
    let dir = tempdir()?;
    let config_path = dir.path().join(local_fs::CONFIG_FILE);
    let iri = "urn:kpar:test";
    let source = OverrideSource::LocalSrc {
        src_path: "local/test".into(),
    };

    local_fs::add_project_source_to_config(&config_path, iri, &source)?;

    let config = Config {
        indexes: vec![],
        projects: vec![ConfigProject {
            identifiers: vec![iri.to_owned()],
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
    let source = OverrideSource::LocalSrc {
        src_path: "local/test".into(),
    };
    let config = Config {
        indexes: vec![],
        projects: vec![ConfigProject {
            identifiers: vec![iri.to_owned()],
            sources: vec![source],
        }],
    };
    config_file.write_all(toml::to_string_pretty(&config)?.as_bytes())?;

    local_fs::remove_project_source_from_config(&config_path, iri)?;

    assert!(!config_path.is_file());

    Ok(())
}
