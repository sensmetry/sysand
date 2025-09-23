// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::{Path, PathBuf};

use pyo3::prelude::*;
use sysand_core::{
    add::do_add,
    build::do_build_kpar,
    commands::{env::do_env_local_dir, new::do_new_local_file},
    env::{
        ReadEnvironment as _, WriteEnvironment,
        local_directory::{DEFAULT_ENV_NAME, LocalDirectoryEnvironment, LocalReadError},
        utils::clone_project,
    },
    exclude::do_exclude,
    include::do_include,
    info::{InfoError, do_info, do_info_project},
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectRead as _,
        local_kpar::LocalKParProject,
        local_src::{LocalSrcError, LocalSrcProject},
    },
    remove::do_remove,
    resolve::standard::standard_resolver,
    sources::{do_sources_local_src_project_no_deps, find_project_dependencies},
    symbols::Language,
};

#[pyfunction(name = "do_new_py_local_file")]
#[pyo3(
    signature = (name, version, path),
)]
fn do_new_py_local_file(name: String, version: String, path: String) -> PyResult<()> {
    do_new_local_file(name, version, std::path::Path::new(&path)).map_err(|err| match err {
        sysand_core::new::NewError::AlreadyExists(msg) => {
            pyo3::exceptions::PyFileExistsError::new_err(msg)
        }
        sysand_core::new::NewError::SemVerError(error) => {
            pyo3::exceptions::PyValueError::new_err(error.to_string())
        }
        sysand_core::new::NewError::ProjectError(err) => match err {
            LocalSrcError::AlreadyExists(msg) => pyo3::exceptions::PyFileExistsError::new_err(msg),
            LocalSrcError::Serde(error) => {
                pyo3::exceptions::PyValueError::new_err(error.to_string())
            }
            LocalSrcError::Io(error) => pyo3::exceptions::PyIOError::new_err(error.to_string()),
            LocalSrcError::Path(path_error) => {
                pyo3::exceptions::PyIOError::new_err(path_error.to_string())
            }
        },
    })?;

    Ok(())
}

#[pyfunction(name = "do_env_py_local_dir")]
#[pyo3(
    signature = (path),
)]
fn do_env_py_local_dir(path: String) -> PyResult<()> {
    do_env_local_dir(Path::new(&path)).map_err(|err| match err {
        sysand_core::commands::env::EnvError::AlreadyExists(path_buf) => {
            pyo3::exceptions::PyFileExistsError::new_err(format!("{}", path_buf.display()))
        }
        sysand_core::commands::env::EnvError::WriteError(werr) => match werr {
            sysand_core::env::local_directory::LocalWriteError::IOError(error) => {
                pyo3::exceptions::PyIOError::new_err(error.to_string())
            }
            sysand_core::env::local_directory::LocalWriteError::SerialisationError(error) => {
                pyo3::exceptions::PyValueError::new_err(error.to_string())
            }
            sysand_core::env::local_directory::LocalWriteError::PathError(path_error) => {
                pyo3::exceptions::PyValueError::new_err(path_error.to_string())
            }
            sysand_core::env::local_directory::LocalWriteError::AlreadyExists(error) => {
                pyo3::exceptions::PyIOError::new_err(error.to_string())
            }
        },
    })?;

    Ok(())
}

#[pyfunction(name = "do_info_py_path")]
#[pyo3(
    signature = (path),
)]
fn do_info_py_path(
    path: String,
) -> PyResult<Option<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)>> {
    let project = LocalSrcProject {
        project_path: Path::new(&path).to_path_buf(),
    };

    Ok(do_info_project(project))
}

#[pyfunction(name = "do_info_py")]
#[pyo3(
    signature = (uri, relative_file_root, index_urls),
)]
fn do_info_py(
    py: Python,
    uri: String,
    relative_file_root: String,
    index_urls: Option<Vec<String>>,
) -> PyResult<Vec<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)>> {
    py.allow_threads(|| {
        let mut results = vec![];
        let client = reqwest::blocking::ClientBuilder::new()
            .build()
            .expect("internal HTTP error");

        let index_url = index_urls
            .map(|url_strs| {
                url_strs
                    .iter()
                    .map(|url_str| url::Url::parse(url_str))
                    .collect()
            })
            .transpose()
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;

        let combined_resolver = standard_resolver(
            Some(std::path::Path::new(&relative_file_root).to_path_buf()),
            None,
            Some(client),
            index_url,
        );

        match do_info(&uri, &combined_resolver) {
            Ok(matches) => results.extend(matches),
            Err(InfoError::NoResolve(_)) => {}
            Err(InfoError::ResolutionError(err)) => {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(err.to_string()));
            }
        };

        Ok(results)
    })
}

#[pyfunction(name = "do_build_py")]
#[pyo3(
    signature = (output_path, project_path),
)]
fn do_build_py(output_path: String, project_path: Option<String>) -> PyResult<()> {
    let Some(current_project_path) = project_path else {
        return Err(pyo3::exceptions::PyNotImplementedError::new_err("TODO"));
    };
    let project = LocalSrcProject {
        project_path: Path::new(&current_project_path).to_path_buf(),
    };

    do_build_kpar(&project, &output_path, true)
        .map(|_| ())
        .map_err(|err| match err {
            sysand_core::build::KParBuildError::ProjectReadError(_) => {
                pyo3::exceptions::PyRuntimeError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::SrcError(_) => {
                pyo3::exceptions::PyRuntimeError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::IncompleteSourceError(_) => {
                pyo3::exceptions::PyRuntimeError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::IOError(_) => {
                pyo3::exceptions::PyIOError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::ValidationError(_) => {
                pyo3::exceptions::PyValueError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::ExtractError(_) => {
                pyo3::exceptions::PyValueError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::UnknownFormat(_) => {
                pyo3::exceptions::PyValueError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::MissingInfo => {
                pyo3::exceptions::PyValueError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::MissingMeta => {
                pyo3::exceptions::PyValueError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::ZipWriteError(_) => {
                pyo3::exceptions::PyIOError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::PathFailure(_) => {
                pyo3::exceptions::PyIOError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::FileNameError => {
                pyo3::exceptions::PyRuntimeError::new_err(err.to_string())
            }
            sysand_core::build::KParBuildError::SerdeError(_) => {
                pyo3::exceptions::PyValueError::new_err(err.to_string())
            }
        })
}

#[pyfunction(name = "do_sources_env_py")]
#[pyo3(
    signature = (env_path, iri, version, include_deps),
)]
pub fn do_sources_env_py(
    env_path: String,
    iri: String,
    version: Option<String>,
    include_deps: bool,
) -> PyResult<Vec<String>> {
    let version = match version {
        Some(version) => Some(
            semver::VersionReq::parse(&version)
                .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?,
        ),
        None => None,
    };

    let mut result = vec![];

    let env = LocalDirectoryEnvironment {
        environment_path: env_path.into(),
    };

    fn local_read_to_pyerr(e: LocalReadError) -> PyErr {
        match e {
            sysand_core::env::local_directory::LocalReadError::IOError(error) => {
                pyo3::exceptions::PyIOError::new_err(error.to_string())
            }
        }
    }

    let mut projects = env
        .candidate_projects(&iri)
        .map_err(local_read_to_pyerr)?
        .into_iter();

    let Some(project) = (match &version {
        None => projects.next(),
        Some(vr) => loop {
            if let Some(candidate) = projects.next() {
                if let Some(v) = candidate
                    .version()
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
                    .and_then(|x| semver::Version::parse(&x).ok())
                {
                    if vr.matches(&v) {
                        break Some(candidate);
                    }
                }
            } else {
                break None;
            }
        },
    }) else {
        match version {
            Some(vr) => {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                    "unable to find project {} ({}) in local environment",
                    iri, vr
                )));
            }
            None => {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                    "unable to find project {} in local environment",
                    iri
                )));
            }
        }
    };

    for src_path in do_sources_local_src_project_no_deps(&project, true)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
    {
        result.push(
            src_path
                .to_str()
                .ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("invalid path".to_string())
                })?
                .to_string(),
        );
    }

    if include_deps {
        let Some(info) = project
            .get_info()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
        else {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "project is missing project information",
            ));
        };

        for dep in find_project_dependencies(
            info.validate()
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
                .usage,
            env,
        )
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
        {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
            {
                result.push(
                    src_path
                        .to_str()
                        .ok_or_else(|| {
                            pyo3::exceptions::PyRuntimeError::new_err("invalid path".to_string())
                        })?
                        .to_string(),
                );
            }
        }
    }

    Ok(result)
}

#[pyfunction(name = "do_sources_project_py")]
#[pyo3(
    signature = (path, include_deps, env_path),
)]
pub fn do_sources_project_py(
    path: String,
    include_deps: bool,
    env_path: Option<String>,
) -> PyResult<Vec<String>> {
    let mut result = vec![];

    let current_project = LocalSrcProject {
        project_path: path.into(),
    };

    for src_path in do_sources_local_src_project_no_deps(&current_project, true)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
    {
        result.push(
            src_path
                .to_str()
                .ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("invalid path".to_string())
                })?
                .to_string(),
        );
    }

    if include_deps {
        // TODO: Better bail early?
        let Some(info) = current_project
            .get_info()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
        else {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "project is missing project information",
            ));
        };

        let Some(env_path) = env_path else {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "Unable to identify local environment",
            ));
        };

        let env = LocalDirectoryEnvironment {
            environment_path: env_path.into(),
        };

        for dep in find_project_dependencies(
            info.validate()
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
                .usage,
            env,
        )
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
        {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
            {
                result.push(
                    src_path
                        .to_str()
                        .ok_or_else(|| {
                            pyo3::exceptions::PyRuntimeError::new_err("invalid path".to_string())
                        })?
                        .to_string(),
                );
            }
        }
    }

    Ok(result)
}

#[pyfunction(name = "do_add_py")]
#[pyo3(
    signature = (path, iri, version),
)]
fn do_add_py(path: String, iri: String, version: Option<String>) -> PyResult<()> {
    let mut project = LocalSrcProject {
        project_path: Path::new(&path).to_path_buf(),
    };

    do_add(&mut project, iri, version)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
}

#[pyfunction(name = "do_remove_py")]
#[pyo3(
    signature = (path, iri),
)]
fn do_remove_py(path: String, iri: String) -> PyResult<()> {
    let mut project = LocalSrcProject {
        project_path: Path::new(&path).to_path_buf(),
    };

    do_remove(&mut project, iri)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

    Ok(())
}

#[pyfunction(name = "do_include_py")]
#[pyo3(
    signature = (path, src_path, compute_checksum, index_symbols, force_format),
)]
fn do_include_py(
    path: String,
    src_path: String,
    compute_checksum: bool,
    index_symbols: bool,
    force_format: Option<String>,
) -> PyResult<()> {
    let mut project = LocalSrcProject {
        project_path: Path::new(&path).to_path_buf(),
    };

    let force_format = match force_format {
        Some(language_str) => match Language::from_suffix(&language_str) {
            Some(language) => Some(language),
            None => {
                return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                    "invalid language identifier: {}",
                    language_str
                )));
            }
        },
        None => None,
    };

    do_include(
        &mut project,
        src_path,
        compute_checksum,
        index_symbols,
        force_format,
    )
    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
}

#[pyfunction(name = "do_exclude_py")]
#[pyo3(
    signature = (path, src_path),
)]
fn do_exclude_py(path: String, src_path: String) -> PyResult<()> {
    let mut project = LocalSrcProject {
        project_path: Path::new(&path).to_path_buf(),
    };

    do_exclude(&mut project, src_path)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

    Ok(())
}

#[pyfunction(name = "do_env_install_path_py")]
#[pyo3(
    signature = (env_path, iri, location),
)]
fn do_env_install_path_py(env_path: String, iri: String, location: String) -> PyResult<()> {
    let project_path: PathBuf = location.clone().into();

    let mut env = LocalDirectoryEnvironment {
        environment_path: env_path.into(),
    };

    if project_path.is_file() {
        let project = LocalKParProject::new_guess_root(project_path)?;

        let Some(version) = project
            .version()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
        else {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "project at {} lacks project information",
                location
            )));
        };

        env.put_project(iri, version, |to| clone_project(&project, to, true))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    } else if project_path.is_dir() {
        let project = LocalSrcProject { project_path };

        let Some(version) = project
            .version()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
        else {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "project at {} lacks project information",
                location
            )));
        };

        env.put_project(iri, version, |to| clone_project(&project, to, true))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    } else {
        return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "unable to find project at {}",
            location
        )));
    }

    Ok(())
}

#[pymodule(name = "_sysand_core")]
pub fn sysand_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    pyo3_log::init();

    m.add_function(wrap_pyfunction!(do_new_py_local_file, m)?)?;
    m.add_function(wrap_pyfunction!(do_env_py_local_dir, m)?)?;
    m.add_function(wrap_pyfunction!(do_info_py_path, m)?)?;
    m.add_function(wrap_pyfunction!(do_info_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_build_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_sources_env_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_sources_project_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_add_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_remove_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_include_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_exclude_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_env_install_path_py, m)?)?;

    m.add("DEFAULT_ENV_NAME", DEFAULT_ENV_NAME)?;
    Ok(())
}
