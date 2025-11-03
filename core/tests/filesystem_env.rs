// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(feature = "filesystem")]
mod filesystem_tests {
    use std::{
        io::{Cursor, Read},
        path::Path,
    };

    use chrono::DateTime;
    use indexmap::IndexMap;
    use semver::Version;
    use sysand_core::{
        commands::env::do_env_local_dir,
        env::{ReadEnvironment, WriteEnvironment, utils::clone_project},
        info::do_info,
        model::{InterchangeProjectInfo, InterchangeProjectMetadata},
        project::{ProjectMut, ProjectRead, memory::InMemoryProject},
        resolve::env::EnvResolver,
    };
    use tempfile::TempDir;
    use typed_path::Utf8UnixPath;

    fn ls_dir<P: AsRef<Path>>(path: P) -> Vec<String> {
        let path = path.as_ref().to_path_buf().clone();

        let mut files: Vec<String> = std::fs::read_dir(&path)
            .unwrap()
            .map(|entry| {
                let inner_path = entry.unwrap().path();

                assert!(inner_path.is_dir() || inner_path.is_file());

                inner_path
                    .strip_prefix(&path)
                    .unwrap()
                    .as_os_str()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        files.sort();

        files
    }

    #[test]
    fn env_basic() -> Result<(), Box<dyn std::error::Error>> {
        let cwd = TempDir::new()?;
        let env_path = Path::new(sysand_core::env::local_directory::DEFAULT_ENV_NAME);
        let directory_environment = do_env_local_dir(cwd.path().join(env_path))?;

        for entry in std::fs::read_dir(cwd.path())? {
            let path = entry?.path();

            assert!(path.is_dir() || path.is_file());

            if path.is_dir() {
                assert_eq!(path.strip_prefix(&cwd)?, env_path);
            } else {
                // if path.is_file()
                assert_eq!(path.strip_prefix(&cwd)?, env_path.join("entries.txt"));
            }
        }

        assert_eq!(
            std::fs::File::open(cwd.path().join("sysand_env/entries.txt"))?
                .metadata()?
                .len(),
            0
        );

        let installed: Result<Vec<String>, sysand_core::env::local_directory::LocalReadError> =
            directory_environment.uris()?.collect();
        let installed = installed?;

        assert_eq!(installed.len(), 0);

        Ok(())
    }

    #[test]
    fn env_manual_install() -> Result<(), Box<dyn std::error::Error>> {
        let cwd = TempDir::new()?;
        let env_path = Path::new(sysand_core::env::local_directory::DEFAULT_ENV_NAME);
        let mut directory_environment = do_env_local_dir(cwd.path().join(env_path))?;

        let info = InterchangeProjectInfo {
            name: "env_manual_install".to_string(),
            description: None,
            version: Version::new(1, 2, 3),
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        }
        .into();

        let mut index = IndexMap::new();
        index.insert(
            "SomePackage".to_string(),
            Utf8UnixPath::new("SomePackage.sysml").to_path_buf(),
        );

        let meta = InterchangeProjectMetadata {
            index,
            created: DateTime::from_timestamp(1, 2).unwrap(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }
        .into();

        let mut source_project = InMemoryProject::default();

        source_project.put_project(&info, &meta, true)?;

        let source_path = Utf8UnixPath::new("SomePackage.sysml");
        let source_code = "package SomePackage;";

        source_project.write_source(source_path, &mut Cursor::new(source_code), true)?;

        directory_environment.put_project("urn::sysand_test::1", "1.2.3", |p| {
            clone_project(&source_project, p, true)
        })?;

        let target_project = directory_environment.get_project("urn::sysand_test::1", "1.2.3")?;

        let (read_info, read_meta) = target_project.get_project()?;

        assert_eq!(read_info, Some(info.clone()));
        assert_eq!(read_meta, Some(meta.clone()));

        let mut read_source_code = "".to_string();

        target_project
            .read_source(source_path)?
            .read_to_string(&mut read_source_code)?;

        assert_eq!(read_source_code, source_code);

        assert_eq!(
            directory_environment
                .versions("urn::sysand_test::1")?
                .collect::<Result<Vec<String>, _>>()?,
            vec!["1.2.3"]
        );
        assert_eq!(
            directory_environment
                .uris()?
                .collect::<Result<Vec<String>, _>>()?,
            vec!["urn::sysand_test::1"]
        );

        assert_eq!(ls_dir(cwd.path()), vec!["sysand_env"]);
        assert_eq!(
            ls_dir(cwd.path().join("sysand_env")),
            vec![
                "8378fe229eabf3f24fa9c102e60e06bf8ee6120c7fda477ccc7fd9705e388fea",
                "entries.txt"
            ]
        );
        assert_eq!(
            ls_dir(
                cwd.path()
                    .join("sysand_env")
                    .join("8378fe229eabf3f24fa9c102e60e06bf8ee6120c7fda477ccc7fd9705e388fea")
            ),
            vec!["1.2.3.kpar", "versions.txt"]
        );
        assert_eq!(
            ls_dir(
                cwd.path()
                    .join("sysand_env")
                    .join("8378fe229eabf3f24fa9c102e60e06bf8ee6120c7fda477ccc7fd9705e388fea")
                    .join("1.2.3.kpar")
            ),
            vec![".meta.json", ".project.json", "SomePackage.sysml"]
        );

        let resolver = EnvResolver {
            env: directory_environment,
        };

        let resolved_projects = do_info("urn::sysand_test::1", &resolver)?;

        assert_eq!(resolved_projects.len(), 1);
        assert_eq!(resolved_projects[0], (info, meta));

        Ok(())
    }
}
