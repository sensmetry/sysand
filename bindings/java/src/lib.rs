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
};

use crate::{
    conversion::{ToJObject, ToJObjectArray},
    exceptions::{ExceptionKind, Throw},
};

mod conversion;
mod exceptions;

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_sysand_Sysand_init__Ljava_lang_String_2Ljava_lang_String_2Ljava_lang_String_2<
    'local,
>(
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
pub extern "system" fn Java_org_sysand_Sysand_defaultEnvName<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    env.new_string(local_directory::DEFAULT_ENV_NAME)
        .expect("Failed to create String")
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_sysand_Sysand_env__Ljava_lang_String_2<'local>(
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
pub extern "system" fn Java_org_sysand_Sysand_info_1path__Ljava_lang_String_2<'local>(
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
pub extern "system" fn Java_org_sysand_Sysand_info<'local>(
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
