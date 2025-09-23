// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{path::PathBuf, sync::Arc};

use jni::{
    JNIEnv,
    objects::{JClass, JObject, JObjectArray, JString},
};
use sysand_core::{
    commands,
    env::local_directory::{self, LocalWriteError},
    info::InfoError,
    new::NewError,
    project::local_src::{LocalSrcError, LocalSrcProject},
    resolve::standard::standard_resolver,
    workspace::Workspace,
};

use crate::{
    conversion::{ToJObject, ToJObjectArray},
    exceptions::{ExceptionKind, StdlibExceptionKind, Throw},
};

mod conversion;
mod exceptions;

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_init<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    name: JString<'local>,
    version: JString<'local>,
    path: JString<'local>,
) {
    let name: String = env.get_string(&name).expect("Failed to get name").into();
    let version: String = env
        .get_string(&version)
        .expect("Failed to get version")
        .into();
    let path: String = env.get_string(&path).expect("Failed to get path").into();
    let command_result = commands::new::do_new_local_file(name, version, path);
    match command_result {
        Ok(_) => {}
        Err(error) => match error {
            NewError::SemVerParse(..) => {
                env.throw_exception(ExceptionKind::InvalidSemanticVersion, error.to_string())
            }
            NewError::Project(suberror) => match suberror {
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
            },
        },
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_defaultEnvName<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    env.new_string(local_directory::DEFAULT_ENV_NAME)
        .expect("Failed to create String")
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_env<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) {
    let path: String = env.get_string(&path).expect("Failed to get path").into();
    let command_result = commands::env::do_env_local_dir(path);
    match command_result {
        Ok(_) => {}
        Err(error) => match error {
            commands::env::EnvError::AlreadyExists(msg) => env.throw_exception(
                ExceptionKind::PathError,
                format!("Path already exists: {}", msg.display()),
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
    let path: String = env.get_string(&path).expect("Failed to get path").into();
    let project = LocalSrcProject {
        project_path: PathBuf::from(&path),
    };

    let command_result = commands::info::do_info_project(&project);
    match command_result {
        Some(info_metadata) => info_metadata.to_jobject(&mut env),
        None => JObject::null(),
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
    let uri: String = env.get_string(&uri).expect("Failed to get uri").into();
    let client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build();

    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("failed to initialise async runtime"),
    );

    let relative_file_root: String = env
        .get_string(&relative_file_root)
        .expect("Failed to get relative file root")
        .into();

    let index_base_url = if index_url.is_null() {
        None
    } else {
        let index_url: String = env
            .get_string(&index_url)
            .expect("Failed to get index url")
            .into();
        match url::Url::parse(&index_url) {
            Ok(url) => Some(url),
            Err(error) => {
                let exception_class = env
                    .find_class("java/lang/UnsupportedOperationException")
                    .expect("Failed to find UnsupportedOperationException class");
                env.throw_new(
                    exception_class,
                    format!("Failed to parse index URL {}: {}", index_url, error),
                )
                .expect("Failed to throw UnsupportedOperationException");
                return JObjectArray::default();
            }
        }
    };

    let combined_resolver = standard_resolver(
        Some(PathBuf::from(&relative_file_root)),
        None,
        Some(client),
        index_base_url.map(|x| vec![x]),
        runtime,
    );

    let results = match commands::info::do_info(&uri, &combined_resolver) {
        Ok(matches) => matches,
        Err(InfoError::NoResolve(_)) => Vec::new(),
        Err(e @ InfoError::Resolution(_)) => {
            env.throw_exception(ExceptionKind::ResolutionError, e.to_string());
            return JObjectArray::default();
        }
    };

    results.to_jobject_array(&mut env)
}

fn handle_build_error(
    env: &mut JNIEnv<'_>,
    error: sysand_core::build::KParBuildError<LocalSrcError>,
) {
    match error {
        sysand_core::build::KParBuildError::ProjectRead(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Project read error: {}", error),
            );
        }
        sysand_core::build::KParBuildError::LocalSrc(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Local src error: {}", error),
            );
        }
        sysand_core::build::KParBuildError::IncompleteSource(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Incomplete source error: {}", error),
            );
        }
        sysand_core::build::KParBuildError::Io(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("IO error: {}", error),
            );
        }
        sysand_core::build::KParBuildError::Validation(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Validation error: {}", error),
            );
        }
        sysand_core::build::KParBuildError::Extract(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Extract error: {}", error),
            );
        }
        sysand_core::build::KParBuildError::UnknownFormat(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Unknown format error: {}", error),
            );
        }
        sysand_core::build::KParBuildError::MissingInfo => {
            env.throw_exception(ExceptionKind::SysandException, "Missing info".to_string());
        }
        sysand_core::build::KParBuildError::MissingMeta => {
            env.throw_exception(ExceptionKind::SysandException, "Missing meta".to_string());
        }
        sysand_core::build::KParBuildError::Zip(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Zip write error: {}", error),
            );
        }
        sysand_core::build::KParBuildError::Serialize(msg, error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Project serialization error: {}: {}", msg, error),
            );
        }
        sysand_core::build::KParBuildError::WorkspaceRead(error) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Workspace read error: {}", error),
            );
        }
        sysand_core::build::KParBuildError::InternalError(error) => {
            env.throw_exception(ExceptionKind::SysandException, error.to_string());
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_buildProject<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    output_path: JString<'local>,
    project_path: JString<'local>,
) {
    let output_path: String = env
        .get_string(&output_path)
        .expect("Failed to get output path")
        .into();
    let project_path: String = env
        .get_string(&project_path)
        .expect("Failed to get project path")
        .into();
    let project = LocalSrcProject {
        project_path: std::path::PathBuf::from(project_path),
    };
    let command_result = sysand_core::commands::build::do_build_kpar(&project, &output_path, true);
    match command_result {
        Ok(_) => {}
        Err(error) => handle_build_error(&mut env, error),
    }
}

fn get_string<'local>(
    env: &mut JNIEnv<'local>,
    string: &JString<'local>,
    variable_name: &str,
) -> Option<String> {
    match env.get_string(string) {
        Ok(string) => Some(string.into()),
        Err(error) => match error {
            jni::errors::Error::WrongJValueType(expected, actual) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::IllegalArgument,
                    format!(
                        "{}: wrong JValue type, expected {:?}, got {:?}",
                        variable_name, expected, actual
                    ),
                );
                None
            }
            jni::errors::Error::InvalidCtorReturn => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::IllegalState,
                    format!("{}: invalid constructor return", variable_name),
                );
                None
            }
            jni::errors::Error::InvalidArgList(type_signature) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::IllegalArgument,
                    format!(
                        "{}: invalid argument list, type signature: {}",
                        variable_name, type_signature
                    ),
                );
                None
            }
            jni::errors::Error::MethodNotFound { name, sig } => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::UnsupportedOperation,
                    format!(
                        "{}: method not found: {} with signature {}",
                        variable_name, name, sig
                    ),
                );
                None
            }
            jni::errors::Error::FieldNotFound { name, sig } => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::UnsupportedOperation,
                    format!(
                        "{}: field not found: {} with signature {}",
                        variable_name, name, sig
                    ),
                );
                None
            }
            jni::errors::Error::JavaException => {
                // A Java exception was already thrown, let it propagate
                None
            }
            jni::errors::Error::JNIEnvMethodNotFound(method_name) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::Runtime,
                    format!(
                        "{}: JNI environment method not found: {}",
                        variable_name, method_name
                    ),
                );
                None
            }
            jni::errors::Error::NullPtr(_) | jni::errors::Error::NullDeref(_) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::NullPointer,
                    format!("{} is null", variable_name),
                );
                None
            }
            jni::errors::Error::TryLock => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::IllegalState,
                    format!("{}: failed to acquire lock", variable_name),
                );
                None
            }
            jni::errors::Error::JavaVMMethodNotFound(method_name) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::Runtime,
                    format!(
                        "{}: Java VM method not found: {}",
                        variable_name, method_name
                    ),
                );
                None
            }
            jni::errors::Error::FieldAlreadySet(field_name) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::IllegalState,
                    format!("{}: field already set: {}", variable_name, field_name),
                );
                None
            }
            jni::errors::Error::ThrowFailed(error_msg) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::Runtime,
                    format!(
                        "{}: failed to throw exception: {}",
                        variable_name, error_msg
                    ),
                );
                None
            }
            jni::errors::Error::ParseFailed(string_stream_error, _) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::IllegalArgument,
                    format!("{}: parse failed: {}", variable_name, string_stream_error),
                );
                None
            }
            jni::errors::Error::JniCall(jni_error) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::Runtime,
                    format!("{}: JNI call failed: {}", variable_name, jni_error),
                );
                None
            }
        },
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sensmetry_sysand_Sysand_buildWorkspace<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    output_path: JString<'local>,
    workspace_path: JString<'local>,
) {
    let Some(output_path) = get_string(&mut env, &output_path, "output_path") else {
        return;
    };
    let Some(workspace_path) = get_string(&mut env, &workspace_path, "workspace_path") else {
        return;
    };
    let workspace = Workspace {
        workspace_path: std::path::PathBuf::from(workspace_path),
    };
    match std::fs::create_dir_all(&output_path) {
        Ok(_) => {}
        Err(error) => {
            env.throw_exception(ExceptionKind::IOError, error.to_string());
            return;
        }
    }
    let command_result =
        sysand_core::commands::build::do_build_workspace_kpars(&workspace, &output_path, true);
    match command_result {
        Ok(_) => {}
        Err(error) => handle_build_error(&mut env, error),
    }
}
