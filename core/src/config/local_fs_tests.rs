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
