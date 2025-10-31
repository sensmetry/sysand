// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{Deserialize, Serialize};

use thiserror::Error;

use crate::{env::ReadEnvironment, project::ProjectRead};

pub const CURRENT_LOCK_VERSION: &str = "0.1";

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Lock {
    pub lock_version: String,
    #[serde(rename = "project", skip_serializing_if = "Vec::is_empty", default)]
    pub project: Vec<Project>,
}

impl Default for Lock {
    fn default() -> Self {
        Lock {
            lock_version: CURRENT_LOCK_VERSION.to_string(),
            project: vec![],
        }
    }
}

#[derive(Error, Debug)]
pub enum LockResolutionEror<EnvironmentError> {
    #[error(transparent)]
    Environment(EnvironmentError),
    #[error("missing projects:\n{0:?}")]
    MissingProjects(Vec<Project>),
}

impl Lock {
    pub fn resolve_projects<Env: ReadEnvironment>(
        &self,
        env: &Env,
    ) -> Result<
        Vec<<Env as ReadEnvironment>::InterchangeProjectRead>,
        LockResolutionEror<Env::ReadError>,
    > {
        let mut missing = vec![];
        let mut found = vec![];

        for project in &self.project {
            let checksum = &project.checksum;

            let mut resolved_project = None;

            'outer: for iri in &project.iris {
                for candidate_project in env
                    .candidate_projects(iri)
                    .map_err(LockResolutionEror::Environment)?
                {
                    if let Ok(Some(candidate_checksum)) = candidate_project.checksum_canonical_hex()
                    {
                        if candidate_checksum == *checksum {
                            resolved_project = Some(candidate_project);
                            break 'outer;
                        }
                    }
                }
            }

            if let Some(success) = resolved_project {
                found.push(success);
            } else {
                missing.push(project.clone());
            }
        }

        if !missing.is_empty() {
            return Err(LockResolutionEror::MissingProjects(missing));
        }

        Ok(found)
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Project {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub info: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub meta: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub iris: Vec<String>,
    pub checksum: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub specification: Option<String>,
    #[serde(rename = "source", skip_serializing_if = "Vec::is_empty", default)]
    pub sources: Vec<Source>,
}

impl Project {
    pub fn name(&self) -> Option<String> {
        if let Some(info) = &self.info {
            Some(info.as_object()?.get("name")?.as_str()?.to_string())
        } else {
            None
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum Source {
    Editable {
        editable: String,
    },
    LocalSrc {
        src_path: String,
    },
    LocalKpar {
        kpar_path: String,
    },
    Registry {
        registry: String,
    },
    RemoteKpar {
        remote_kpar: String,
        remote_kpar_size: Option<u64>,
    },
    RemoteSrc {
        remote_src: String,
    },
    RemoteGit {
        remote_git: String,
    },
    RemoteApi {
        remote_api: String,
    },
}

#[test]
fn toml_example() {
    let json_1 = serde_json::from_str(
        r#"
    { "name": "foobar", "version": "1.2.3" }
    "#,
    )
    .unwrap();

    let meta_1 = serde_json::from_str(
        r#"
    null
    "#,
    )
    .unwrap();

    let json_2 = serde_json::from_str(
        r#"
    null
    "#,
    )
    .unwrap();

    let meta_2 = serde_json::from_str(
        r#"
    { "created": "1871-03-18T12:00:00.000000000+01:00" }
    "#,
    )
    .unwrap();

    let example = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        project: vec![
            Project {
                info: json_1,
                meta: meta_1,
                iris: vec![
                    "ftp://www.example.com".to_string(),
                    "http://www.example.com".to_string(),
                ],
                checksum: "FF".to_string(),
                specification: None,
                sources: vec![Source::Editable {
                    editable: ".".to_string(),
                }],
            },
            Project {
                info: json_2,
                meta: meta_2,
                iris: vec![],
                checksum: "00".to_string(),
                specification: Some("example".to_string()),
                sources: vec![],
            },
        ],
    };

    let expected = format!(
        r#"lock_version = "{}"

[[project]]
iris = ["ftp://www.example.com", "http://www.example.com"]
checksum = "FF"

[project.info]
name = "foobar"
version = "1.2.3"

[[project.source]]
editable = "."

[[project]]
checksum = "00"
specification = "example"

[project.meta]
created = "1871-03-18T12:00:00.000000000+01:00"
"#,
        CURRENT_LOCK_VERSION
    );

    assert_eq!(toml::to_string(&example).unwrap(), expected.to_string());
}
