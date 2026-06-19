// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{ffi::c_void, sync::Arc};

use camino::Utf8PathBuf;
use fluent_uri::Iri;
use jni::{
    Env, JNIVersion, NativeMethod,
    errors::Result as JniResult,
    jni_str, native_method,
    objects::{JClass, JObject, JObjectArray, JString},
    sys::{JNI_ERR, jint},
    vm::JavaVM,
};
use sysand_core::{
    auth::Unauthenticated,
    commands,
    env::{DEFAULT_ENV_NAME, local_directory::LocalWriteError},
    init::InitError,
    project::{
        ProjectMut,
        local_src::{LocalSrcError, LocalSrcProject},
        utils::wrapfs,
    },
    resolve::{net_utils::create_reqwest_client, standard::standard_resolver},
    utils::format_err,
    workspace::Workspace,
};

use crate::{
    conversion::{
        ToJObject, ToJStringArray, compression_from_java_string, handle_build_error,
        java_info_to_raw, java_map_to_index_map, java_metadata_to_raw,
    },
    exceptions::{ExceptionKind, JniExt, StdlibExceptionKind},
};

mod conversion;
mod exceptions;

macro_rules! ret {
    () => {
        return Ok(())
    };
}

macro_rules! ret_null {
    () => {
        return Ok(Default::default())
    };
}

// Using `native_method!` requires that function returns `Result<T, jni::errors::Error>`,
// but we already throw our own exceptions, and changing to return errors instead would
// lose all error context. So the return type matches what is required, but we never
// actually return `Err()`
const SYSAND_INIT: NativeMethod = native_method!(
    static fn init(name: JString, publisher: JString, version: JString, license: JString, path: JString)
);
fn init<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    name: JString<'local>,
    publisher: JString<'local>,
    version: JString<'local>,
    license: JString<'local>,
    path: JString<'local>,
) -> JniResult<()> {
    let Some(name) = env.get_str(&name, "name") else {
        ret!()
    };
    let Some(publisher) = env.get_str(&publisher, "publisher") else {
        ret!()
    };
    let Some(version) = env.get_str(&version, "version") else {
        ret!()
    };
    let Some(path) = env.get_str(&path, "path") else {
        ret!()
    };

    // If `license` is `null`, no license is specified
    let license: Option<String> = match license.mutf8_chars(env) {
        Ok(s) => Some(s.into()),
        Err(e) => match e {
            jni::errors::Error::NullPtr(_) => None,
            _ => {
                env.throw_runtime_exception(format!(
                    "failed to get argument `license`: {}",
                    format_err(e)
                ));
                ret!()
            }
        },
    };

    let command_result =
        commands::init::do_init_local_file(name, publisher, version, license, path.into());
    match command_result {
        Ok(_) => Ok(()),
        Err(error) => {
            let e = format_err(&error);
            match error {
                InitError::SemVerParse(..) => {
                    env.throw_exception(ExceptionKind::InvalidSemanticVersion, e)
                }
                InitError::SPDXLicenseParse(..) => {
                    env.throw_exception(ExceptionKind::InvalidSPDXLicense, e)
                }
                InitError::Project(suberror) => match suberror {
                    LocalSrcError::AlreadyExists(_) => {
                        env.throw_exception(ExceptionKind::ProjectAlreadyExists, e)
                    }
                    LocalSrcError::Deserialize(_) => {
                        env.throw_exception(ExceptionKind::InvalidValue, e)
                    }
                    LocalSrcError::Io(_) => env.throw_exception(ExceptionKind::IOError, e),
                    LocalSrcError::Path(_) => env.throw_exception(ExceptionKind::PathError, e),
                    LocalSrcError::Serialize(_) => {
                        env.throw_exception(ExceptionKind::SerializationError, e)
                    }
                    LocalSrcError::ImpossibleRelativePath(_) => {
                        env.throw_exception(ExceptionKind::PathError, e)
                    }
                    LocalSrcError::MissingMeta | LocalSrcError::MissingInfoMeta => {
                        env.throw_exception(ExceptionKind::SysandException, e)
                    }
                    LocalSrcError::PublisherMismatch { .. }
                    | LocalSrcError::NameMismatch { .. } => {
                        env.throw_exception(ExceptionKind::ResolutionError, e)
                    }
                },
            }
            Ok(())
        }
    }
}

const SYSAND_DEFAULT_ENV_NAME: NativeMethod = native_method!(
    static fn default_env_name() -> JString
);
fn default_env_name<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
) -> JniResult<JString<'local>> {
    match env.new_string(DEFAULT_ENV_NAME) {
        Ok(s) => Ok(s),
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to create String: {}", format_err(e)));
            ret_null!()
        }
    }
}

const SYSAND_ENV: NativeMethod = native_method!(
    static fn create_env(path: JString),
    name = "env"
);
// Not `env` because of name clashes inside `native_method!`. Export name still `env`
fn create_env<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) -> JniResult<()> {
    let Some(path) = env.get_str(&path, "path") else {
        ret!()
    };
    let command_result = commands::env::do_env_local_dir(path);
    match command_result {
        Ok(_) => Ok(()),
        Err(error) => {
            let e = format_err(&error);
            match error {
                commands::env::EnvError::AlreadyExists(path) => env.throw_exception(
                    ExceptionKind::PathError,
                    format!("Path already exists: {path}"),
                ),
                commands::env::EnvError::Write(suberror) => match suberror {
                    LocalWriteError::Io(_) => env.throw_exception(ExceptionKind::IOError, e),
                    LocalWriteError::Deserialize(_) => {
                        env.throw_exception(ExceptionKind::InvalidValue, e)
                    }
                    LocalWriteError::Path(_) => env.throw_exception(ExceptionKind::PathError, e),
                    LocalWriteError::AlreadyExists(_) => {
                        env.throw_exception(ExceptionKind::IOError, e)
                    }
                    LocalWriteError::Serialize(_) => {
                        env.throw_exception(ExceptionKind::SerializationError, e)
                    }
                    LocalWriteError::TryMove(_) => env.throw_exception(ExceptionKind::IOError, e),
                    LocalWriteError::LocalRead(_) => env.throw_exception(ExceptionKind::IOError, e),
                    LocalWriteError::ImpossibleRelativePath(_) => {
                        env.throw_exception(ExceptionKind::PathError, e)
                    }
                    LocalWriteError::AddProject(_) => {
                        env.throw_exception(ExceptionKind::IOError, e)
                    }
                    LocalWriteError::MissingMeta
                    | LocalWriteError::MissingInfoMeta
                    | LocalWriteError::PublisherMismatch { .. }
                    | LocalWriteError::NameMismatch { .. } => {
                        env.throw_exception(ExceptionKind::SysandException, e)
                    }
                },
            }
            Ok(())
        }
    }
}

const SYSAND_INFO_PATH: NativeMethod = native_method!(
    static fn info_path(path: JString) -> com.sensmetry.sysand.model.InterchangeProject
);
fn info_path<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) -> JniResult<JObject<'local>> {
    let Some(path) = env.get_str(&path, "path") else {
        ret_null!()
    };
    let project = LocalSrcProject::new_access(Utf8PathBuf::from(&path), None);

    let command_result = commands::info::do_info_project(&project);
    match command_result {
        Ok(info_metadata) => {
            if let Some(project) = info_metadata.to_jobject(env) {
                return Ok(project);
            }
        }
        Err(e) => {
            env.throw_exception(ExceptionKind::SysandException, format_err(e));
        }
    }
    ret_null!()
}

const SYSAND_INFO: NativeMethod = native_method!(
    static fn info(uri: JString, index_url: JString) -> com.sensmetry.sysand.model.InterchangeProject
);
fn info<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    uri: JString<'local>,
    index_url: JString<'local>,
) -> JniResult<JObject<'local>> {
    let Some(uri) = env.get_str(&uri, "uri") else {
        ret_null!()
    };
    let client = match create_reqwest_client() {
        Ok(c) => c,
        Err(e) => {
            env.throw_exception(ExceptionKind::SysandException, format_err(e));
            ret_null!()
        }
    };

    let runtime = {
        let r = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                env.throw_exception(
                    ExceptionKind::IOError,
                    format!("Failed to build tokio runtime: {e}"),
                );
                ret_null!()
            }
        };
        Arc::new(r)
    };

    let index_base_url = if index_url.is_null() {
        None
    } else {
        let Some(index_url) = env.get_str(&index_url, "indexUrl") else {
            ret_null!()
        };
        match sysand_core::index_location::IndexLocation::parse(&index_url) {
            Ok(location) => Some(location),
            Err(error) => {
                env.throw_stdlib_exception(
                    StdlibExceptionKind::UnsupportedOperationException,
                    format!("Failed to parse index URL `{}`: {}", index_url, error),
                );
                ret_null!()
            }
        }
    };

    let combined_resolver = match standard_resolver(
        None,
        Some(client),
        index_base_url.map(|x| vec![x]),
        runtime,
        // FIXME: Add Java support for authentication
        Arc::new(Unauthenticated {}),
    ) {
        Ok(resolver) => resolver,
        Err(error) => {
            env.throw_exception(
                ExceptionKind::ResolutionError,
                format!("Failed to discover index endpoints: {}", format_err(error)),
            );
            ret_null!()
        }
    };

    let uri = match Iri::parse(uri) {
        Ok(u) => u,
        Err((error, input)) => {
            env.throw_exception(
                ExceptionKind::ResolutionError,
                format!("Provided IRI `{input}` is invalid: {error}"),
            );
            ret_null!()
        }
    };
    let info_meta = match commands::info::do_info(&uri, &combined_resolver) {
        Ok(info_meta) => info_meta,
        Err(e) => {
            env.throw_exception(ExceptionKind::ResolutionError, format_err(e));
            ret_null!()
        }
    };

    Ok(info_meta.to_jobject(env).unwrap_or_default())
}

const SYSAND_WORKSPACE_PROJECT_PATHS: NativeMethod = native_method!(
    static fn workspace_project_paths(workspace_path: JString) -> JString[]
);
fn workspace_project_paths<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    workspace_path: JString<'local>,
) -> JniResult<JObjectArray<'local, JString<'local>>> {
    let Some(workspace_path) = env.get_str(&workspace_path, "workspacePath") else {
        ret_null!()
    };
    let workspace = match Workspace::new(workspace_path.into()) {
        Ok(w) => w,
        Err(e) => {
            env.throw_exception(ExceptionKind::InvalidWorkspace, format_err(e));
            ret_null!()
        }
    };
    let paths: Vec<String> = workspace
        .absolute_project_paths()
        .into_iter()
        .map(|p| p.into_string())
        .collect();
    Ok(paths.to_jstring_array(env).unwrap_or_default())
}

const SYSAND_SET_PROJECT_INDEX: NativeMethod = native_method!(
    static fn set_project_index(project_path: JString, index: java.util.LinkedHashMap)
);
fn set_project_index<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    project_path: JString<'local>,
    index: JObject<'local>,
) -> JniResult<()> {
    let Some(project_path) = env.get_str(&project_path, "projectPath") else {
        ret!()
    };
    let Some(rust_index) = java_map_to_index_map(env, index) else {
        ret!()
    };
    let mut project = LocalSrcProject::new_access(Utf8PathBuf::from(project_path), None);
    let _ = project
        .set_index(rust_index)
        .inspect_err(|e| env.throw_exception(ExceptionKind::SysandException, format_err(e)));
    Ok(())
}

const SYSAND_SET_PROJECT_INFO: NativeMethod = native_method!(
    static fn set_project_info(project_path: JString, info: com.sensmetry.sysand.model.InterchangeProjectInfo)
);
fn set_project_info<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    project_path: JString<'local>,
    info: JObject<'local>,
) -> JniResult<()> {
    let Some(project_path) = env.get_str(&project_path, "projectPath") else {
        ret!()
    };
    let Some(info_raw) = java_info_to_raw(env, &info) else {
        ret!()
    };
    let mut project = LocalSrcProject::new_access(Utf8PathBuf::from(project_path), None);
    let _ = project
        .put_info(&info_raw, true)
        .inspect_err(|e| env.throw_exception(ExceptionKind::SysandException, format_err(e)));
    Ok(())
}

const SYSAND_SET_PROJECT_METADATA: NativeMethod = native_method!(
    static fn set_project_metadata(project_path: JString, metadata: com.sensmetry.sysand.model.InterchangeProjectMetadata)
);
fn set_project_metadata<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    project_path: JString<'local>,
    metadata: JObject<'local>,
) -> JniResult<()> {
    let Some(project_path) = env.get_str(&project_path, "projectPath") else {
        ret!()
    };
    let Some(metadata_raw) = java_metadata_to_raw(env, &metadata) else {
        ret!()
    };
    let mut project = LocalSrcProject::new_access(Utf8PathBuf::from(project_path), None);
    let _ = project
        .put_meta(&metadata_raw, true)
        .inspect_err(|e| env.throw_exception(ExceptionKind::SysandException, format_err(e)));
    Ok(())
}

const SYSAND_BUILD_PROJECT: NativeMethod = native_method!(
    static fn build_project(output_path: JString, project_path: JString, compression: JString)
);
fn build_project<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    output_path: JString<'local>,
    project_path: JString<'local>,
    compression: JString<'local>,
) -> JniResult<()> {
    let Some(output_path) = env.get_str(&output_path, "outputPath") else {
        ret!()
    };
    let Some(project_path) = env.get_str(&project_path, "projectPath") else {
        ret!()
    };
    let project = LocalSrcProject::new_access(Utf8PathBuf::from(project_path), None);
    let Some(compression) = env.get_str(&compression, "compression") else {
        ret!()
    };
    let Some(compression) = compression_from_java_string(env, compression) else {
        ret!()
    };
    let command_result = sysand_core::commands::build::do_build_kpar(
        &project,
        &output_path,
        compression,
        // Currently keeping index updating disabled, since users can set their own index,
        // and flipping this to true would overwrite that potentially custom index.
        // TODO: add this as argument
        false,
        true,
    );
    match command_result {
        Ok(_) => (),
        Err(error) => handle_build_error(env, error),
    }
    Ok(())
}

const SYSAND_BUILD_WORKSPACE: NativeMethod = native_method!(
    static fn build_workspace(output_path: JString, workspace_path: JString, compression: JString)
);
fn build_workspace<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    output_path: JString<'local>,
    workspace_path: JString<'local>,
    compression: JString<'local>,
) -> JniResult<()> {
    let Some(output_path) = env.get_str(&output_path, "outputPath") else {
        ret!()
    };
    let Some(workspace_path) = env.get_str(&workspace_path, "workspacePath") else {
        ret!()
    };
    let workspace = match Workspace::new(workspace_path.into()) {
        Ok(w) => w,
        Err(e) => {
            env.throw_exception(ExceptionKind::InvalidWorkspace, format_err(e));
            ret!()
        }
    };
    let Some(compression) = env.get_str(&compression, "compression") else {
        ret!()
    };
    let Some(compression) = compression_from_java_string(env, compression) else {
        ret!()
    };
    match wrapfs::create_dir_all(&output_path) {
        Ok(_) => {}
        Err(e) => {
            env.throw_exception(ExceptionKind::IOError, format_err(e));
            ret!()
        }
    }

    let command_result = sysand_core::commands::build::do_build_workspace_kpars(
        &workspace,
        &output_path,
        compression,
        // Currently keeping index updating disabled, since users can set their own index,
        // and flipping this to true would overwrite that potentially custom index.
        // TODO: add this as argument
        false,
        true,
    );
    match command_result {
        Ok(_) => {}
        Err(error) => handle_build_error(env, error),
    }
    Ok(())
}

// Test-only hook: the declaring Java class `SysandTestHooks` lives in the
// java-test sources, not in the published jar, so this entry point is not
// reachable through the public Java API. For the same reason, it can't be
// registered in JNI_OnLoad, since the test classes are not included in
// the release; instead specifying `extern` exports the correctly mangled
// function name for the JVM to find
const _: NativeMethod = native_method!(
    java_type = com.sensmetry.sysand.SysandTestHooks,
    static extern fn model_roundtrip(info: com.sensmetry.sysand.model.InterchangeProjectInfo, metadata: com.sensmetry.sysand.model.InterchangeProjectMetadata) -> com.sensmetry.sysand.model.InterchangeProject
);
fn model_roundtrip<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    info: JObject<'local>,
    metadata: JObject<'local>,
) -> JniResult<JObject<'local>> {
    let Some(info) = java_info_to_raw(env, &info) else {
        ret_null!()
    };
    let Some(metadata) = java_metadata_to_raw(env, &metadata) else {
        ret_null!()
    };
    Ok((info, metadata).to_jobject(env).unwrap_or_default())
}

/// `JNI_OnLoad` is automatically called by the JVM when the library
/// is loaded. Do not call this manually.
///
/// # Safety
///
/// `vm_ptr` must be a pointer to a valid initialized JVM
#[unsafe(no_mangle)]
pub unsafe extern "system" fn JNI_OnLoad(
    vm_ptr: *mut jni::sys::JavaVM,
    _reserved: *mut c_void,
) -> jint {
    // SAFETY: Caller is responsible for providing a valid pointer
    let vm = unsafe { JavaVM::from_raw(vm_ptr) };

    let registration_result = vm.attach_current_thread(|env| {
        let sysand = env.load_class(jni_str!("com.sensmetry.sysand.Sysand"))?;
        // SAFETY: `native_method!()` checks signatures to match intended on the Rust side,
        //         and also checks that `class`/`this` arguments are correct.
        //         Function names/args might not match those in Java, these will be checked
        //         and error out on mismatch.
        unsafe {
            env.register_native_methods(
                sysand,
                &[
                    SYSAND_INIT,
                    SYSAND_BUILD_PROJECT,
                    SYSAND_BUILD_WORKSPACE,
                    SYSAND_DEFAULT_ENV_NAME,
                    SYSAND_ENV,
                    SYSAND_INFO,
                    SYSAND_INFO_PATH,
                    SYSAND_SET_PROJECT_INDEX,
                    SYSAND_SET_PROJECT_INFO,
                    SYSAND_SET_PROJECT_METADATA,
                    SYSAND_WORKSPACE_PROJECT_PATHS,
                ],
            )?;
        }
        JniResult::Ok(())
    });

    match registration_result {
        Ok(()) => {
            env_logger::init();
            JNIVersion::V1_8.into()
        }
        Err(e) => {
            eprintln!(
                "error: failed to attach to Java VM or register native methods:\n{}",
                format_err(e)
            );
            // Registration failed, return JNI_ERR to abort library loading
            JNI_ERR
        }
    }
}
