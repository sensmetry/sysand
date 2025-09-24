// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use jni::{
    JNIEnv,
    objects::{JClass, JObject, JObjectArray, JString},
};
use sysand_core::{
    info::InfoError,
    project::local_src::{LocalSrcError, LocalSrcProject},
    resolve::standard::standard_resolver,
};

use crate::{
    conversion::{ToJObject, ToJObjectArray},
    exceptions::throw_exception,
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
    let command_result = sysand_core::commands::new::do_new_local_file(name, version, path);
    match command_result {
        Ok(_) => {}
        Err(error) => match error {
            sysand_core::new::NewError::AlreadyExists(msg) => {
                throw_exception(&mut env, "ProjectAlreadyExists", msg)
            }
            sysand_core::new::NewError::SemVerError(suberror) => {
                throw_exception(&mut env, "InvalidSemanticVersion", suberror.to_string())
            }
            sysand_core::new::NewError::ProjectError(suberror) => match suberror {
                LocalSrcError::AlreadyExists(msg) => {
                    throw_exception(&mut env, "ProjectAlreadyExists", msg)
                }
                LocalSrcError::Serde(subsuberror) => {
                    throw_exception(&mut env, "InvalidValue", subsuberror.to_string())
                }
                LocalSrcError::Io(subsuberror) => {
                    throw_exception(&mut env, "IOError", subsuberror.to_string())
                }
                LocalSrcError::Path(subsuberror) => {
                    throw_exception(&mut env, "PathError", subsuberror.to_string())
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
    env.new_string(sysand_core::env::local_directory::DEFAULT_ENV_NAME)
        .expect("Failed to create String")
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_sysand_Sysand_env__Ljava_lang_String_2<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) {
    let path: String = env.get_string(&path).expect("Failed to get path").into();
    let command_result = sysand_core::commands::env::do_env_local_dir(path);
    match command_result {
        Ok(_) => {}
        Err(error) => match error {
            sysand_core::commands::env::EnvError::AlreadyExists(msg) => throw_exception(
                &mut env,
                "EnvironmentAlreadyExists",
                format!("Path already exists: {}", msg.display()),
            ),
            sysand_core::commands::env::EnvError::WriteError(suberror) => match suberror {
                sysand_core::env::local_directory::LocalWriteError::IOError(subsuberror) => {
                    throw_exception(&mut env, "IOError", subsuberror.to_string())
                }
                sysand_core::env::local_directory::LocalWriteError::SerialisationError(
                    subsuberror,
                ) => throw_exception(&mut env, "SerialisationError", subsuberror.to_string()),
                sysand_core::env::local_directory::LocalWriteError::PathError(subsuberror) => {
                    throw_exception(&mut env, "PathError", subsuberror.to_string())
                }
                sysand_core::env::local_directory::LocalWriteError::AlreadyExists(msg) => {
                    throw_exception(&mut env, "IOError", msg)
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
        project_path: std::path::PathBuf::from(&path),
    };

    let command_result = sysand_core::commands::info::do_info_project(project);
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
    let client = reqwest::blocking::ClientBuilder::new()
        .build()
        .expect("internal HTTP error");

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
        Some(std::path::PathBuf::from(&relative_file_root)),
        None,
        Some(client),
        index_base_url,
    );

    let results = match sysand_core::commands::info::do_info(&uri, &combined_resolver) {
        Ok(matches) => matches,
        Err(InfoError::NoResolve(_)) => Vec::new(),
        Err(InfoError::ResolutionError(error)) => {
            throw_exception(&mut env, "ResolutionError", error.to_string());
            return JObjectArray::default();
        }
    };

    results.to_jobject_array(&mut env)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_sysand_Sysand_build__Ljava_lang_String_2Ljava_lang_String_2<
    'local,
>(
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
        Err(error) => match error {
            sysand_core::build::KParBuildError::ProjectReadError(_) => todo!(),
            sysand_core::build::KParBuildError::SrcError(local_src_error) => todo!(),
            sysand_core::build::KParBuildError::IncompleteSourceError(_) => todo!(),
            sysand_core::build::KParBuildError::IOError(error) => todo!(),
            sysand_core::build::KParBuildError::ValidationError(
                interchange_project_validation_error,
            ) => todo!(),
            sysand_core::build::KParBuildError::ExtractError(extract_error) => todo!(),
            sysand_core::build::KParBuildError::UnknownFormat(_) => todo!(),
            sysand_core::build::KParBuildError::MissingInfo => todo!(),
            sysand_core::build::KParBuildError::MissingMeta => todo!(),
            sysand_core::build::KParBuildError::ZipWriteError(zip_error) => todo!(),
            sysand_core::build::KParBuildError::PathFailure(_) => todo!(),
            sysand_core::build::KParBuildError::FileNameError => todo!(),
            sysand_core::build::KParBuildError::SerdeError(error) => todo!(),
        },
    }
}
