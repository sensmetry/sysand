// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use camino::Utf8PathBuf;
use jni::{
    JNIEnv,
    errors::Error,
    objects::{JClass, JObject, JObjectArray, JString},
};
use sysand_core::{
    auth::Unauthenticated,
    build::{KParBuildError, KparCompressionMethod},
    commands,
    env::local_directory::{self, LocalWriteError},
    info::InfoError,
    init::InitError,
    project::{
        local_src::{LocalSrcError, LocalSrcProject},
        utils::wrapfs,
    },
    resolve::{net_utils::create_reqwest_client, standard::standard_resolver},
    workspace::Workspace,
};

use crate::{
    conversion::{ToJObject, ToJObjectArray, java_map_to_index_map},
    exceptions::{ExceptionKind, JniExt, StdlibExceptionKind},
};

mod conversion;
mod exceptions;

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_init<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    name: JString<'local>,
    publisher: JString<'local>,
    version: JString<'local>,
    license: JString<'local>,
    path: JString<'local>,
) {
    let Some(name) = env.get_str(&name, "name") else {
        return;
    };
    // If `publisher` is `null`, no publisher is specified
    let publisher: Option<String> = match env.get_string(&publisher) {
        Ok(s) => Some(s.into()),
        Err(e) => match e {
            Error::NullPtr(_) => None,
            _ => {
                env.throw_runtime_exception(format!("failed to get argument `publisher`: {}", e));
                return;
            }
        },
    };
    let Some(version) = env.get_str(&version, "version") else {
        return;
    };
    let Some(path) = env.get_str(&path, "path") else {
        return;
    };

    // If `license` is `null`, no license is specified
    let license: Option<String> = match env.get_string(&license) {
        Ok(s) => Some(s.into()),
        Err(e) => match e {
            Error::NullPtr(_) => None,
            _ => {
                env.throw_runtime_exception(format!("failed to get argument `license`: {}", e));
                return;
            }
        },
    };

    let command_result =
        commands::init::do_init_local_file(name, publisher, version, license, path.into());
    match command_result {
        Ok(_) => {}
        Err(error) => match error {
            InitError::SemVerParse(..) => {
                env.throw_exception(ExceptionKind::InvalidSemanticVersion, error.to_string())
            }
            InitError::SPDXLicenseParse(..) => {
                env.throw_exception(ExceptionKind::InvalidSPDXLicense, error.to_string())
            }
            InitError::Project(suberror) => match suberror {
                LocalSrcError::AlreadyExists(msg) => {
                    env.throw_exception(ExceptionKind::ProjectAlreadyExists, msg)
                }
                LocalSrcError::Deserialize(subsuberror) => {
                    env.throw_exception(ExceptionKind::InvalidValue, subsuberror.to_string())
                }
                LocalSrcError::Io(subsuberror) => {
                    env.throw_exception(ExceptionKind::IOError, subsuberror.to_string())
                }
                LocalSrcError::Path(subsuberror) => {
                    env.throw_exception(ExceptionKind::PathError, subsuberror.to_string())
                }
                LocalSrcError::Serialize(subsuberror) => {
                    env.throw_exception(ExceptionKind::SerializationError, subsuberror.to_string())
                }
                LocalSrcError::ImpossibleRelativePath(_) => {
                    env.throw_exception(ExceptionKind::PathError, suberror.to_string())
                }
                LocalSrcError::MissingMeta => {
                    env.throw_exception(ExceptionKind::SysandException, suberror.to_string())
                }
            },
        },
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_defaultEnvName<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    match env.new_string(local_directory::DEFAULT_ENV_NAME) {
        Ok(s) => s,
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to create String: {e}"));
            JString::default()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_env<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) {
    let Some(path) = env.get_str(&path, "path") else {
        return;
    };
    let command_result = commands::env::do_env_local_dir(path);
    match command_result {
        Ok(_) => {}
        Err(error) => match error {
            commands::env::EnvError::AlreadyExists(path) => env.throw_exception(
                ExceptionKind::PathError,
                format!("Path already exists: {}", path),
            ),
            commands::env::EnvError::Write(suberror) => match suberror {
                LocalWriteError::Io(subsuberror) => {
                    env.throw_exception(ExceptionKind::IOError, subsuberror.to_string())
                }
                LocalWriteError::Deserialize(subsuberror) => {
                    env.throw_exception(ExceptionKind::InvalidValue, subsuberror.to_string())
                }
                LocalWriteError::Path(subsuberror) => {
                    env.throw_exception(ExceptionKind::PathError, subsuberror.to_string())
                }
                LocalWriteError::AlreadyExists(msg) => {
                    env.throw_exception(ExceptionKind::IOError, msg)
                }
                LocalWriteError::Serialize(subsuberror) => {
                    env.throw_exception(ExceptionKind::SerializationError, subsuberror.to_string())
                }
                LocalWriteError::TryMove(subsuberror) => {
                    env.throw_exception(ExceptionKind::IOError, subsuberror.to_string())
                }
                LocalWriteError::LocalRead(subsuberror) => {
                    env.throw_exception(ExceptionKind::IOError, subsuberror.to_string())
                }
                LocalWriteError::ImpossibleRelativePath(_) => {
                    env.throw_exception(ExceptionKind::PathError, suberror.to_string())
                }
                LocalWriteError::MissingMeta => {
                    env.throw_exception(ExceptionKind::SysandException, suberror.to_string())
                }
            },
        },
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_infoPath<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) -> JObject<'local> {
    let Some(path) = env.get_str(&path, "path") else {
        return JObject::default();
    };
    let project = LocalSrcProject {
        nominal_path: None,
        project_path: Utf8PathBuf::from(&path),
    };

    let command_result = commands::info::do_info_project(&project);
    match command_result {
        Some(info_metadata) => info_metadata.to_jobject(&mut env).unwrap_or_default(),
        None => JObject::default(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_info<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    uri: JString<'local>,
    relative_file_root: JString<'local>,
    index_url: JString<'local>,
) -> JObjectArray<'local> {
    let Some(uri) = env.get_str(&uri, "uri") else {
        return JObjectArray::default();
    };
    let client = match create_reqwest_client() {
        Ok(c) => c,
        Err(e) => {
            env.throw_exception(ExceptionKind::SysandException, e.to_string());
            return JObjectArray::default();
        }
    };

    let runtime = {
        let r = match tokio::runtime::Builder::new_current_thread().build() {
            Ok(r) => r,
            Err(e) => {
                env.throw_exception(
                    ExceptionKind::IOError,
                    format!("Failed to build tokio runtime: {e}"),
                );
                return JObjectArray::default();
            }
        };
        Arc::new(r)
    };

    let Some(relative_file_root) = env.get_str(&relative_file_root, "relativeFileRoot") else {
        return JObjectArray::default();
    };

    let index_base_url = if index_url.is_null() {
        None
    } else {
        let Some(index_url) = env.get_str(&index_url, "indexUrl") else {
            return JObjectArray::default();
        };
        match url::Url::parse(&index_url) {
            Ok(url) => Some(url),
            Err(error) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::UnsupportedOperationException,
                    format!("Failed to parse index URL `{}`: {}", index_url, error),
                );
                return JObjectArray::default();
            }
        }
    };

    let combined_resolver = standard_resolver(
        Some(Utf8PathBuf::from(relative_file_root)),
        None,
        Some(client),
        index_base_url.map(|x| vec![x]),
        runtime,
        // FIXME: Add Java support for authentication
        Arc::new(Unauthenticated {}),
    );

    let results = match commands::info::do_info(&uri, &combined_resolver) {
        Ok(matches) => matches,
        Err(InfoError::NoResolve(..)) => Vec::new(),
        Err(e @ (InfoError::UnsupportedIri(..) | InfoError::Resolution(_))) => {
            env.throw_exception(ExceptionKind::ResolutionError, e.to_string());
            return JObjectArray::default();
        }
    };

    results.to_jobject_array(&mut env).unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_workspaceProjectPaths<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    workspace_path: JString<'local>,
) -> JObjectArray<'local> {
    let Some(workspace_path) = env.get_str(&workspace_path, "workspacePath") else {
        return JObjectArray::default();
    };
    let workspace = match Workspace::new(workspace_path.into()) {
        Ok(w) => w,
        Err(e) => {
            env.throw_exception(ExceptionKind::InvalidWorkspace, e.to_string());
            return JObjectArray::default();
        }
    };
    let paths: Vec<String> = workspace
        .absolute_project_paths()
        .into_iter()
        .map(|p| p.into_string())
        .collect();
    paths.to_jobject_array(&mut env).unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_setProjectIndex<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    project_path: JString<'local>,
    index: JObject<'local>,
) {
    let Some(project_path) = env.get_str(&project_path, "projectPath") else {
        return;
    };
    let rust_index = match java_map_to_index_map(&mut env, &index) {
        Ok(index) => index,
        Err(jni::errors::Error::JavaException) => {
            // Exception already thrown by get_str
            return;
        }
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to convert index map: {e}"));
            return;
        }
    };
    let mut project = LocalSrcProject {
        nominal_path: None,
        project_path: Utf8PathBuf::from(project_path),
    };
    let _ = project
        .set_index(rust_index)
        .inspect_err(|e| env.throw_exception(ExceptionKind::SysandException, e.to_string()));
}

fn handle_build_error(env: &mut JNIEnv<'_>, error: KParBuildError<LocalSrcError>) {
    match error {
        KParBuildError::ProjectRead(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Project read error: {}", error),
            );
        }
        KParBuildError::LocalSrc(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Local src error: {}", error),
            );
        }
        KParBuildError::IncompleteSource(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Incomplete source error: {}", error),
            );
        }
        KParBuildError::Io(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("IO error: {}", error),
            );
        }
        KParBuildError::Validation(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Validation error: {}", error),
            );
        }
        KParBuildError::Extract(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Extract error: {}", error),
            );
        }
        KParBuildError::UnknownFormat(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Unknown format error: {}", error),
            );
        }
        KParBuildError::MissingInfo => {
            env.throw_exception(
                ExceptionKind::SysandException,
                "Missing project information",
            );
        }
        KParBuildError::MissingMeta => {
            env.throw_exception(ExceptionKind::SysandException, "Missing project metadata");
        }
        KParBuildError::Zip(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Zip write error: {}", error),
            );
        }
        KParBuildError::Serialize(msg, error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Project serialization error: {}: {}", msg, error),
            );
        }
        KParBuildError::WorkspaceRead(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Workspace read error: {}", error),
            );
        }
        KParBuildError::PathUsage(usage) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!(
                    "project includes a path usage `{usage}`,\n\
        which is unlikely to be available on other computers at the same path"
                ),
            );
        }
        KParBuildError::WorkspaceMetamodelConflict { .. } => {
            env.throw_exception(ExceptionKind::SysandException, error.to_string());
        }
    }
}

fn compression_from_java_string(
    env: &mut JNIEnv<'_>,
    compression: String,
) -> Option<KparCompressionMethod> {
    match KparCompressionMethod::try_from(compression) {
        Ok(compression) => Some(compression),
        Err(err) => {
            env.throw_exception(ExceptionKind::SysandException, err.to_string());
            None
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_buildProject<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    output_path: JString<'local>,
    project_path: JString<'local>,
    compression: JString<'local>,
) {
    let Some(output_path) = env.get_str(&output_path, "outputPath") else {
        return;
    };
    let Some(project_path) = env.get_str(&project_path, "projectPath") else {
        return;
    };
    let project = LocalSrcProject {
        nominal_path: None,
        project_path: Utf8PathBuf::from(project_path),
    };
    let Some(compression) = env.get_str(&compression, "compression") else {
        return;
    };
    let Some(compression) = compression_from_java_string(&mut env, compression) else {
        return;
    };
    let command_result = sysand_core::commands::build::do_build_kpar(
        &project,
        &output_path,
        compression,
        true,
        false,
    );
    match command_result {
        Ok(_) => {}
        Err(error) => handle_build_error(&mut env, error),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_buildWorkspace<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    output_path: JString<'local>,
    workspace_path: JString<'local>,
    compression: JString<'local>,
) {
    let Some(output_path) = env.get_str(&output_path, "outputPath") else {
        return;
    };
    let Some(workspace_path) = env.get_str(&workspace_path, "workspacePath") else {
        return;
    };
    let workspace = match Workspace::new(workspace_path.into()) {
        Ok(w) => w,
        Err(e) => {
            env.throw_exception(ExceptionKind::InvalidWorkspace, e.to_string());
            return;
        }
    };
    let Some(compression) = env.get_str(&compression, "compression") else {
        return;
    };
    let Some(compression) = compression_from_java_string(&mut env, compression) else {
        return;
    };
    match wrapfs::create_dir_all(&output_path) {
        Ok(_) => {}
        Err(error) => {
            env.throw_exception(ExceptionKind::IOError, error.to_string());
            return;
        }
    }

    let command_result = sysand_core::commands::build::do_build_workspace_kpars(
        &workspace,
        &output_path,
        compression,
        true,
        false,
    );
    match command_result {
        Ok(_) => {}
        Err(error) => handle_build_error(&mut env, error),
    }
}
