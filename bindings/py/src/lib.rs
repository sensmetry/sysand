// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, process::ExitCode, sync::Arc};

use camino::{Utf8Path, Utf8PathBuf};
use pyo3::{
    exceptions::{PyFileExistsError, PyIOError, PyRuntimeError, PyValueError},
    prelude::*,
};
use semver::{Version, VersionReq};
use sysand_core::{
    add::do_add,
    auth::Unauthenticated,
    build::{KParBuildError, do_build_kpar},
    commands::{
        env::{EnvError, do_env_local_dir},
        init::do_init_local_file,
    },
    env::{
        ReadEnvironment as _, WriteEnvironment,
        local_directory::{
            DEFAULT_ENV_NAME, LocalDirectoryEnvironment, LocalReadError, LocalWriteError,
        },
        utils::clone_project,
    },
    exclude::do_exclude,
    include::do_include,
    info::{InfoError, do_info, do_info_project},
    init::InitError,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectRead as _,
        local_kpar::LocalKParProject,
        local_src::{LocalSrcError, LocalSrcProject},
    },
    remove::do_remove,
    resolve::{net_utils::create_reqwest_client, standard::standard_resolver},
    sources::{do_sources_local_src_project_no_deps, find_project_dependencies},
    stdlib::known_std_libs,
    symbols::Language,
};

#[pyfunction(name = "_run_cli")]
fn run_cli(args: Vec<String>) -> PyResult<bool> {
    let exit_code = sysand::lib_main(args);
    Ok(exit_code == ExitCode::SUCCESS)
}

#[pyfunction(name = "do_new_py_local_file")]
#[pyo3(
    signature = (name, version, path, license=None),
)]
fn do_new_py_local_file(
    name: String,
    version: String,
    path: String,
    license: Option<String>,
) -> PyResult<()> {
    // Initialize logger in each function independently to avoid setting up a
    // logger before `run_cli()` is called (CLI sets up its own logger). This
    // can't be put into pymodule definition, since importing any part of the
    // library from python runs it
    let _ = pyo3_log::try_init();

    do_init_local_file(name, version, license, Utf8PathBuf::from(path)).map_err(
        |err| match err {
            InitError::SemVerParse(..) => PyValueError::new_err(err.to_string()),
            InitError::SPDXLicenseParse(..) => PyValueError::new_err(err.to_string()),
            InitError::Project(err) => match err {
                LocalSrcError::AlreadyExists(msg) => PyFileExistsError::new_err(msg),
                LocalSrcError::Deserialize(error) => PyValueError::new_err(error.to_string()),
                LocalSrcError::Io(error) => PyIOError::new_err(error.to_string()),
                LocalSrcError::Path(error) => PyIOError::new_err(error.to_string()),
                LocalSrcError::Serialize(error) => PyValueError::new_err(error.to_string()),
            },
        },
    )?;

    Ok(())
}

#[pyfunction(name = "do_env_py_local_dir")]
#[pyo3(
    signature = (path),
)]
fn do_env_py_local_dir(path: String) -> PyResult<()> {
    let _ = pyo3_log::try_init();

    do_env_local_dir(Utf8Path::new(&path)).map_err(|err| match err {
        EnvError::AlreadyExists(path_buf) => PyFileExistsError::new_err(path_buf.into_string()),
        EnvError::Write(werr) => match werr {
            LocalWriteError::Io(error) => PyIOError::new_err(error.to_string()),
            LocalWriteError::Deserialize(error) => PyValueError::new_err(error.to_string()),
            LocalWriteError::Path(error) => PyValueError::new_err(error.to_string()),
            LocalWriteError::AlreadyExists(error) => PyFileExistsError::new_err(error.to_string()),
            LocalWriteError::Serialize(error) => PyValueError::new_err(error.to_string()),
            LocalWriteError::TryMove(error) => PyIOError::new_err(error.to_string()),
            LocalWriteError::LocalRead(error) => PyIOError::new_err(error.to_string()),
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
    let _ = pyo3_log::try_init();

    let project = LocalSrcProject {
        project_path: path.into(),
    };

    Ok(do_info_project(&project))
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
    let _ = pyo3_log::try_init();

    py.detach(|| {
        let mut results = vec![];
        let client = create_reqwest_client();

        let runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        );

        let index_url = index_urls
            .map(|url_strs| {
                url_strs
                    .iter()
                    .map(|url_str| url::Url::parse(url_str))
                    .collect()
            })
            .transpose()
            .map_err(|err| PyValueError::new_err(err.to_string()))?;

        let combined_resolver = standard_resolver(
            Some(relative_file_root.into()),
            None,
            Some(client),
            index_url,
            runtime,
            // FIXME: Add Python support for authentication
            Arc::new(Unauthenticated {}),
        );

        match do_info(&uri, &combined_resolver) {
            Ok(matches) => results.extend(matches),
            Err(InfoError::NoResolve(_)) => {}
            Err(e @ InfoError::Resolution(_)) => {
                return Err(PyRuntimeError::new_err(e.to_string()));
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
    let _ = pyo3_log::try_init();

    let Some(current_project_path) = project_path else {
        return Err(pyo3::exceptions::PyNotImplementedError::new_err("TODO"));
    };
    let project = LocalSrcProject {
        project_path: current_project_path.into(),
    };

    do_build_kpar(&project, &output_path, true)
        .map(|_| ())
        .map_err(|err| match err {
            KParBuildError::ProjectRead(_) => PyRuntimeError::new_err(err.to_string()),
            KParBuildError::LocalSrc(_) => PyRuntimeError::new_err(err.to_string()),
            KParBuildError::IncompleteSource(_) => PyRuntimeError::new_err(err.to_string()),
            KParBuildError::Io(_) => PyIOError::new_err(err.to_string()),
            KParBuildError::Validation(_) => PyValueError::new_err(err.to_string()),
            KParBuildError::Extract(_) => PyValueError::new_err(err.to_string()),
            KParBuildError::UnknownFormat(_) => PyValueError::new_err(err.to_string()),
            KParBuildError::MissingInfo => PyValueError::new_err(err.to_string()),
            KParBuildError::MissingMeta => PyValueError::new_err(err.to_string()),
            KParBuildError::Zip(_) => PyIOError::new_err(err.to_string()),
            KParBuildError::Serialize(..) => PyValueError::new_err(err.to_string()),
            KParBuildError::WorkspaceRead(_) => PyRuntimeError::new_err(err.to_string()),
            KParBuildError::InternalError(_) => PyRuntimeError::new_err(err.to_string()),
        })
}

#[pyfunction(name = "do_sources_env_py")]
#[pyo3(
    signature = (env_path, iri, version, include_deps, include_std),
)]
pub fn do_sources_env_py(
    env_path: String,
    iri: String,
    version: Option<String>,
    include_deps: bool,
    include_std: bool,
) -> PyResult<Vec<String>> {
    let _ = pyo3_log::try_init();

    let provided_iris = if !include_std {
        known_std_libs()
    } else {
        HashMap::default()
    };

    let version = match version {
        Some(version) => Some(
            VersionReq::parse(&version).map_err(|err| PyValueError::new_err(err.to_string()))?,
        ),
        None => None,
    };

    let mut result = vec![];

    let env = LocalDirectoryEnvironment {
        environment_path: env_path.into(),
    };

    fn local_read_to_pyerr(e: LocalReadError) -> PyErr {
        match e {
            LocalReadError::Io(error) => PyIOError::new_err(error.to_string()),
            LocalReadError::ProjectListFileRead(_) => PyIOError::new_err(e.to_string()),
            LocalReadError::ProjectVersionsFileRead(_) => PyIOError::new_err(e.to_string()),
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
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
                    .and_then(|x| Version::parse(&x).ok())
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
                return Err(PyRuntimeError::new_err(format!(
                    "unable to find project `{}` ({}) in local environment",
                    iri, vr
                )));
            }
            None => {
                return Err(PyRuntimeError::new_err(format!(
                    "unable to find project `{}` in local environment",
                    iri
                )));
            }
        }
    };

    for src_path in do_sources_local_src_project_no_deps(&project, true)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
    {
        result.push(src_path.into_string());
    }

    if include_deps {
        let Some(info) = project
            .get_info()
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
        else {
            return Err(PyRuntimeError::new_err(
                "project is missing project information",
            ));
        };

        for dep in find_project_dependencies(
            info.validate()
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
                .usage,
            env,
            &provided_iris,
        )
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
        {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
            {
                result.push(src_path.into_string());
            }
        }
    }

    Ok(result)
}

#[pyfunction(name = "do_sources_project_py")]
#[pyo3(
    signature = (path, include_deps, env_path, include_std),
)]
pub fn do_sources_project_py(
    path: String,
    include_deps: bool,
    env_path: Option<String>,
    include_std: bool,
) -> PyResult<Vec<String>> {
    let _ = pyo3_log::try_init();

    let mut result = vec![];

    let current_project = LocalSrcProject {
        project_path: path.into(),
    };

    for src_path in do_sources_local_src_project_no_deps(&current_project, true)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
    {
        result.push(src_path.into_string());
    }

    if include_deps {
        // TODO: Better bail early?
        let Some(info) = current_project
            .get_info()
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
        else {
            return Err(PyRuntimeError::new_err(
                "project is missing project information",
            ));
        };

        let Some(env_path) = env_path else {
            return Err(PyRuntimeError::new_err(
                "unable to identify local environment",
            ));
        };

        let provided_iris = if !include_std {
            known_std_libs()
        } else {
            HashMap::default()
        };

        let env = LocalDirectoryEnvironment {
            environment_path: env_path.into(),
        };

        for dep in find_project_dependencies(
            info.validate()
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
                .usage,
            env,
            &provided_iris,
        )
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
        {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
            {
                result.push(src_path.into_string());
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
    let _ = pyo3_log::try_init();

    let mut project = LocalSrcProject {
        project_path: path.into(),
    };

    do_add(&mut project, iri, version).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pyfunction(name = "do_remove_py")]
#[pyo3(
    signature = (path, iri),
)]
fn do_remove_py(path: String, iri: String) -> PyResult<()> {
    let _ = pyo3_log::try_init();

    let mut project = LocalSrcProject {
        project_path: path.into(),
    };

    do_remove(&mut project, iri).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

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
    let _ = pyo3_log::try_init();

    let mut project = LocalSrcProject {
        project_path: path.into(),
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
    .map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pyfunction(name = "do_exclude_py")]
#[pyo3(
    signature = (path, src_path),
)]
fn do_exclude_py(path: String, src_path: String) -> PyResult<()> {
    let _ = pyo3_log::try_init();

    let mut project = LocalSrcProject {
        project_path: path.into(),
    };

    do_exclude(&mut project, src_path).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

    Ok(())
}

#[pyfunction(name = "do_env_install_path_py")]
#[pyo3(
    signature = (env_path, iri, location),
)]
fn do_env_install_path_py(env_path: String, iri: String, location: String) -> PyResult<()> {
    let _ = pyo3_log::try_init();

    let location: Utf8PathBuf = location.into();

    let mut env = LocalDirectoryEnvironment {
        environment_path: env_path.into(),
    };

    if location.is_file() {
        let project = LocalKParProject::new_guess_root(&location)
            .map_err(|e| PyErr::new::<PyIOError, _>(e.to_string()))?;

        let Some(version) = project
            .version()
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
        else {
            return Err(PyRuntimeError::new_err(format!(
                "project at `{}` lacks project information",
                location
            )));
        };

        env.put_project(iri, version, |to| {
            clone_project(&project, to, true).map(|_| ())
        })
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    } else if location.is_dir() {
        let project = LocalSrcProject {
            project_path: location,
        };

        let Some(version) = project
            .version()
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
        else {
            return Err(PyRuntimeError::new_err(format!(
                "project at {} lacks project information",
                project.project_path
            )));
        };

        env.put_project(iri, version, |to| {
            clone_project(&project, to, true).map(|_| ())
        })
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    } else {
        return Err(PyRuntimeError::new_err(format!(
            "unable to find project at `{}`",
            location
        )));
    }

    Ok(())
}

#[pymodule(name = "_sysand_core")]
pub fn sysand_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
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
