// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{Deserialize, Serialize};

use thiserror::Error;
use toml_edit::{
    Array, ArrayOfTables, DocumentMut, Formatted, InlineTable, Item, Table, Value, value,
};

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

    pub fn to_toml(&self) -> DocumentMut {
        let mut doc = DocumentMut::new();
        doc.insert("lock_version", value(Value::from(&self.lock_version)));

        let mut projects = ArrayOfTables::new();
        for project in &self.project {
            projects.push(project.to_toml());
        }
        doc.insert("project", Item::ArrayOfTables(projects));

        doc
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
    pub fn to_toml(&self) -> Table {
        let mut table = Table::new();
        if let Some(info) = &self.info {
            // table.insert("info", value(info.to_string()));
            // TODO: Break up info to avoid this nonsense
            let info_str = toml::to_string(info).unwrap();
            let info_toml = info_str.parse::<DocumentMut>().unwrap();
            table.insert("info", info_toml.as_item().clone());
        }
        if let Some(meta) = &self.meta {
            // table.insert("meta", value(meta.to_string()));
            // TODO: Break up meta to avoid this nonsense
            let meta_str = toml::to_string(meta).unwrap();
            let meta_toml = meta_str.parse::<DocumentMut>().unwrap();
            table.insert("meta", meta_toml.as_item().clone());
        }
        let iris = multiline_array(self.iris.iter().map(Value::from));
        if !iris.is_empty() {
            table.insert("iris", value(iris));
        }
        table.insert("checksum", value(&self.checksum));
        if let Some(specification) = &self.specification {
            table.insert("specification", value(specification));
        }
        let sources = multiline_array(self.sources.iter().map(|s| s.to_toml()));
        if !sources.is_empty() {
            table.insert("sources", value(sources));
        }
        table
    }
}

fn multiline_array(elements: impl Iterator<Item = impl Into<Value>>) -> Array {
    let mut array: Array = elements
        .map(|item| {
            let mut value = item.into();
            value.decor_mut().set_prefix("\n    ");
            value
        })
        .collect();
    array.set_trailing_comma(true);
    array.set_trailing("\n");
    array
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

impl Source {
    pub fn to_toml(&self) -> InlineTable {
        let mut table = InlineTable::new();
        match self {
            Source::Editable { editable } => {
                table.insert("editable", Value::from(editable));
            }
            Source::LocalKpar { kpar_path } => {
                table.insert("kpar_path", Value::from(kpar_path));
            }
            Source::LocalSrc { src_path } => {
                table.insert("src_path", Value::from(src_path));
            }
            Source::Registry { registry } => {
                table.insert("registry", Value::from(registry));
            }
            Source::RemoteApi { remote_api } => {
                table.insert("remote_api", Value::from(remote_api));
            }
            Source::RemoteGit { remote_git } => {
                table.insert("remote_git", Value::from(remote_git));
            }
            Source::RemoteKpar {
                remote_kpar,
                remote_kpar_size,
            } => {
                table.insert("remote_kpar", Value::from(remote_kpar));
                if let Some(remote_kpar_size) = remote_kpar_size {
                    let size = i64::try_from(*remote_kpar_size).unwrap();
                    table.insert("remote_kpar_size", Value::Integer(Formatted::new(size)));
                }
            }
            Source::RemoteSrc { remote_src } => {
                table.insert("remote_src", Value::from(remote_src));
            }
        }
        table
    }
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
