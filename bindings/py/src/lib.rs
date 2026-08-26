// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{iter, process::ExitCode, sync::Arc};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use pyo3::{
    exceptions::{PyFileExistsError, PyFileNotFoundError, PyIOError, PyRuntimeError, PyValueError},
    prelude::*,
    types::PyAny,
};
use semver::{Version, VersionReq};
use sysand_core::{
    add::do_add_guess,
    auth::Unauthenticated,
    build::{KParBuildError, KparCompressionMethod, do_build_kpar},
    commands::{
        env::{EnvError, do_env_local_dir},
        init::do_init_local_file,
    },
    env::{
        DEFAULT_ENV_NAME, ReadEnvironment as _, WriteEnvironment as _,
        local_directory::{
            LocalDirectoryEnvironment, LocalReadError, LocalWriteError, metadata::EnvMetadataError,
        },
        utils::clone_project,
    },
    exclude::do_exclude,
    include::do_include,
    index_location::IndexLocation,
    info::{InfoProjectError, do_info, do_info_project},
    init::InitError,
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsage, InterchangeProjectUsageRaw,
    },
    project::{
        ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::{LocalSrcError, LocalSrcProject},
        utils::wrapfs,
    },
    remove::do_remove_guess,
    resolve::{net_utils::create_reqwest_client, standard::standard_resolver},
    root::do_root,
    sources::{Dependencies, do_sources_local_src_project_no_deps, resolve_dependencies},
    symbols::Language,
    utils::format_err,
};
use typed_path::Utf8UnixPathBuf;

#[pyfunction(name = "_run_cli")]
fn run_cli(args: Vec<String>) -> bool {
    let exit_code;
    // Expand glob arguments, CMD/PowerShell don't do it
    #[cfg(windows)]
    {
        use glob::{MatchOptions, glob_with};
        use std::ffi::OsString;

        let options = MatchOptions {
            case_sensitive: false,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };

        let args = args.into_iter().flat_map(|arg| {
            if !arg.contains(['*', '?']) {
                return vec![arg.into()];
            }

            // Treat '[' and ']' as literal characters to match Windows behavior
            let escaped = arg.replace('[', "[[]");

            match glob_with(&escaped, options) {
                Ok(entries) => {
                    let matches: Vec<OsString> = entries
                        .filter_map(|m| match m {
                            Ok(s) => Some(s.into_os_string()),
                            Err(e) => {
                                // can't use log::warn here, since the logger is likely uninitialized
                                eprintln!("warning: failed to expand pattern: {e}");
                                None
                            }
                        })
                        .collect();

                    if !matches.is_empty() {
                        return matches;
                    }
                }
                Err(e) => eprintln!("warning: invalid pattern `{arg}`: {e}"),
            }

            vec![arg.into()]
        });
        exit_code = sysand::lib_main(args);
    }
    #[cfg(not(windows))]
    {
        exit_code = sysand::lib_main(args);
    }
    exit_code == ExitCode::SUCCESS
}

#[pyfunction(name = "do_init_py_local_file")]
#[pyo3(
    signature = (name, publisher, version, path, license=None),
)]
fn do_init_py_local_file(
    name: String,
    publisher: String,
    version: String,
    path: String,
    license: Option<String>,
) -> PyResult<()> {
    // Initialize logger in each function independently to avoid setting up a
    // logger before `run_cli()` is called (CLI sets up its own logger). This
    // can't be put into pymodule definition, since importing any part of the
    // library from python runs it
    common_init();

    do_init_local_file(name, publisher, version, license, Utf8PathBuf::from(path)).map_err(
        |err| {
            let e = format_err(&err);
            match err {
                InitError::SemVerParse(..) | InitError::SPDXLicenseParse(..) => {
                    PyValueError::new_err(e)
                }
                InitError::Project(err) => match err {
                    LocalSrcError::AlreadyExists(_) => PyFileExistsError::new_err(e),
                    LocalSrcError::Io(_) | LocalSrcError::Path(_) => PyIOError::new_err(e),
                    LocalSrcError::Serialize(_)
                    | LocalSrcError::ImpossibleRelativePath(_)
                    | LocalSrcError::Deserialize(_)
                    | LocalSrcError::PublisherMismatch { .. }
                    | LocalSrcError::NameMismatch { .. } => PyValueError::new_err(e),
                    LocalSrcError::MissingMeta | LocalSrcError::MissingInfoMeta => {
                        PyFileNotFoundError::new_err(e)
                    }
                },
            }
        },
    )?;

    Ok(())
}

#[pyfunction(name = "do_env_py_local_dir")]
#[pyo3(
    signature = (path),
)]
fn do_env_py_local_dir(path: String) -> PyResult<()> {
    common_init();

    do_env_local_dir(Utf8Path::new(&path)).map_err(|err| {
        let e = format_err(&err);
        match err {
            EnvError::AlreadyExists(_) => PyFileExistsError::new_err(e),
            EnvError::Write(werr) => match werr {
                LocalWriteError::AlreadyExists(_) => PyFileExistsError::new_err(e),
                LocalWriteError::Deserialize(_)
                | LocalWriteError::Path(_)
                | LocalWriteError::Serialize(_)
                | LocalWriteError::ImpossibleRelativePath(_)
                | LocalWriteError::PublisherMismatch { .. }
                | LocalWriteError::ProjectNotFound(_)
                | LocalWriteError::NameMismatch { .. } => PyValueError::new_err(e),
                LocalWriteError::Io(_)
                | LocalWriteError::TryMove(_)
                | LocalWriteError::LocalRead(_)
                | LocalWriteError::AddProject(_) => PyIOError::new_err(e),
                LocalWriteError::MissingMeta | LocalWriteError::MissingInfoMeta => {
                    PyFileNotFoundError::new_err(e)
                }
            },
        }
    })?;

    Ok(())
}

#[pyfunction(name = "do_info_py_path")]
#[pyo3(
    signature = (path),
)]
fn do_info_py_path(
    path: String,
) -> PyResult<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
    common_init();

    let project = LocalSrcProject::new_access(path, None);

    match do_info_project(&project) {
        Ok(info_meta) => Ok(info_meta),
        Err(
            e @ (InfoProjectError::MissingProject
            | InfoProjectError::MissingInfo
            | InfoProjectError::MissingMeta
            | InfoProjectError::InvalidProject(..)),
        ) => Err(PyRuntimeError::new_err(format_err(e))),
    }
}

#[pyfunction(name = "do_info_py")]
#[pyo3(
    signature = (uri, index_urls),
)]
fn do_info_py(
    py: Python,
    uri: String,
    index_urls: Option<Vec<String>>,
) -> PyResult<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
    common_init();

    py.detach(|| {
        let client = create_reqwest_client().map_err(|e| PyRuntimeError::new_err(format_err(e)))?;

        let runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        );

        let index_url = index_urls
            .map(|url_strs| {
                url_strs
                    .iter()
                    .map(|url_str| IndexLocation::parse(url_str))
                    .collect()
            })
            .transpose()
            .map_err(|err| PyValueError::new_err(format_err(err)))?;

        let combined_resolver = standard_resolver(
            None,
            Some(client),
            index_url,
            runtime,
            // FIXME: Add Python support for authentication
            Arc::new(Unauthenticated {}),
        )
        .map_err(|err| PyValueError::new_err(format_err(err)))?;

        let uri = Iri::parse(uri)
            .map_err(|(e, input)| PyValueError::new_err(format!("invalid IRI `{input}`: {e}")))?;
        match do_info(&uri, &combined_resolver) {
            Ok(info_meta) => Ok(info_meta),
            Err(e) => Err(PyRuntimeError::new_err(format_err(e))),
        }
    })
}

#[pyfunction(name = "do_root_py")]
#[pyo3(
    signature = (path),
)]
fn do_root_py(path: String) -> PyResult<Option<String>> {
    common_init();

    let root = do_root(Utf8PathBuf::from(path)).map_err(|e| PyIOError::new_err(format_err(e)))?;
    Ok(root.map(Utf8PathBuf::into_string))
}

#[pyfunction(name = "do_build_py")]
#[pyo3(
    signature = (output_path, project_path, compression),
)]
fn do_build_py(
    output_path: String,
    project_path: Option<String>,
    compression: Option<String>,
) -> PyResult<()> {
    common_init();

    let Some(current_project_path) = project_path else {
        return Err(pyo3::exceptions::PyNotImplementedError::new_err("TODO"));
    };
    let project = LocalSrcProject::new_access(current_project_path, None);

    let compression = match compression {
        Some(compression) => match KparCompressionMethod::try_from(compression) {
            Ok(compression) => compression,
            Err(err) => return Err(PyValueError::new_err(format_err(err))),
        },
        None => KparCompressionMethod::default(),
    };

    match do_build_kpar(&project, &output_path, compression, true, true) {
        Ok(_) => Ok(()),
        Err(err) => Err({
            let e = format_err(&err);
            match err {
                KParBuildError::Validation { .. }
                | KParBuildError::Extract(_)
                | KParBuildError::UnknownFormat(_)
                | KParBuildError::MissingInfo
                | KParBuildError::MissingMeta
                | KParBuildError::MissingInfoMeta
                | KParBuildError::Serialize(..)
                | KParBuildError::PathUsage(_)
                | KParBuildError::WorkspaceMetamodelConflict { .. }
                | KParBuildError::MissingIndexSymbol(_, _) => PyValueError::new_err(e),
                KParBuildError::Io(_) | KParBuildError::Zip(_) => PyIOError::new_err(e),
                KParBuildError::ProjectRead(_) | KParBuildError::WorkspaceRead(_) => {
                    PyRuntimeError::new_err(e)
                }
            }
        }),
    }
}

/// Collects the source files of the dependencies of `usages` selected by
/// `dependencies` (resolved in `env`).
fn collect_dependency_sources(
    env: LocalDirectoryEnvironment,
    usages: Vec<InterchangeProjectUsage>,
    dependencies: Dependencies,
) -> PyResult<Vec<String>> {
    let mut result = vec![];
    for dep in resolve_dependencies(usages, env, dependencies)
        .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
    {
        for src_path in do_sources_local_src_project_no_deps(&dep, true)
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
        {
            result.push(src_path.into_string());
        }
    }
    Ok(result)
}

#[pyfunction(name = "do_sources_env_py")]
#[pyo3(
    signature = (env_path, iri, version, no_own, dependencies),
)]
pub fn do_sources_env_py(
    env_path: String,
    iri: String,
    version: Option<String>,
    no_own: bool,
    dependencies: String,
) -> PyResult<Vec<String>> {
    fn local_read_to_pyerr(err: LocalReadError) -> PyErr {
        let e = format_err(&err);
        match err {
            LocalReadError::Io(_) => PyIOError::new_err(e),
            LocalReadError::ProjectNotFound(_) => PyValueError::new_err(e),
        }
    }

    common_init();

    let dependencies = Dependencies::try_from(dependencies.as_str())
        .map_err(|e| PyValueError::new_err(format_err(e)))?;

    let version = match version {
        Some(version) => Some(
            VersionReq::parse(&version).map_err(|err| PyValueError::new_err(format_err(err)))?,
        ),
        None => None,
    };

    let mut result = vec![];

    let env = LocalDirectoryEnvironment::read(&env_path).map_err(env_read_to_pyerr)?;

    let mut projects = env
        .candidate_projects(&iri)
        .map_err(local_read_to_pyerr)?
        .into_iter();

    let Some(project) = (match &version {
        None => projects.next(),
        Some(vr) => loop {
            if let Some(candidate) = projects.next() {
                if let Some(v) = candidate
                    .get_info()
                    .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
                    .and_then(|x| match Version::parse(&x.version) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            log::debug!("ignoring env project `{}` because it has invalid semver version:\n{e}", x.name);
                            None
                        },
                    })
                    && vr.matches(&v)
                {
                    break Some(candidate);
                }
            } else {
                break None;
            }
        },
    }) else {
        match version {
            Some(vr) => {
                return Err(PyRuntimeError::new_err(format!(
                    "unable to find project `{iri}` ({vr}) in local environment"
                )));
            }
            None => {
                return Err(PyRuntimeError::new_err(format!(
                    "unable to find project `{iri}` in local environment"
                )));
            }
        }
    };

    if !no_own {
        for src_path in do_sources_local_src_project_no_deps(&project, true)
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
        {
            result.push(src_path.into_string());
        }
    }

    if dependencies != Dependencies::None {
        let Some(info) = project
            .get_info()
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
        else {
            return Err(PyRuntimeError::new_err(
                "project is missing project information",
            ));
        };

        let usages = info
            .validate()
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
            .usage;

        result.extend(collect_dependency_sources(env, usages, dependencies)?);
    }

    Ok(result)
}

#[pyfunction(name = "do_sources_project_py")]
#[pyo3(
    signature = (path, no_own, dependencies, env_path),
)]
pub fn do_sources_project_py(
    path: String,
    no_own: bool,
    dependencies: String,
    env_path: Option<String>,
) -> PyResult<Vec<String>> {
    common_init();

    let dependencies = Dependencies::try_from(dependencies.as_str())
        .map_err(|e| PyValueError::new_err(format_err(e)))?;

    let mut result = vec![];

    let current_project = LocalSrcProject::new_access(path, None);

    if !no_own {
        for src_path in do_sources_local_src_project_no_deps(&current_project, true)
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
        {
            result.push(src_path.into_string());
        }
    }

    if dependencies != Dependencies::None {
        // TODO: Better bail early?
        let Some(info) = current_project
            .get_info()
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
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

        let env = LocalDirectoryEnvironment::read(&env_path).map_err(env_read_to_pyerr)?;

        let usages = info
            .validate()
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
            .usage;

        result.extend(collect_dependency_sources(env, usages, dependencies)?);
    }

    Ok(result)
}

#[pyfunction(name = "do_add_py")]
#[pyo3(
    signature = (path, iri, version),
)]
fn do_add_py(path: String, iri: String, version: Option<String>) -> PyResult<()> {
    common_init();

    let mut project = LocalSrcProject::new_access(path, None);

    // TODO: do dependency resolution and locking?
    match do_add_guess(&mut project, iri, version) {
        Ok(_added) => Ok(()),
        Err(e) => Err(PyRuntimeError::new_err(format_err(e))),
    }
}

#[pyfunction(name = "do_remove_py")]
#[pyo3(
    signature = (path, iri),
)]
fn do_remove_py(path: String, iri: String) -> PyResult<()> {
    common_init();

    let mut project = LocalSrcProject::new_access(path, None);

    do_remove_guess(&mut project, iri).map_err(|e| PyRuntimeError::new_err(format_err(e)))?;

    Ok(())
}

/// `src_path` must be relative to and under the project root
/// and use Unix separators. No normalization will be performed
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
    common_init();

    let mut project = LocalSrcProject::new_access(path, None);
    let force_format = match force_format {
        Some(language_str) => match Language::from_suffix(&language_str) {
            Some(language) => Some(language),
            None => {
                return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                    "invalid language identifier: {language_str}"
                )));
            }
        },
        None => None,
    };

    do_include(
        &mut project,
        iter::once(Utf8UnixPathBuf::from(src_path)),
        compute_checksum,
        index_symbols,
        force_format,
    )
    .map_err(|e| PyRuntimeError::new_err(format_err(e)))
}

/// `src_path` must be relative to and under the project root
/// and use Unix separators. No normalization will be performed
#[pyfunction(name = "do_exclude_py")]
#[pyo3(
    signature = (path, src_path),
)]
fn do_exclude_py(path: String, src_path: String) -> PyResult<()> {
    common_init();

    let mut project = LocalSrcProject::new_access(path, None);
    // TODO: print the whole error chain
    do_exclude(&mut project, iter::once(Utf8UnixPathBuf::from(src_path)))
        .map_err(|e| PyRuntimeError::new_err(format_err(e)))?;

    Ok(())
}

#[pyfunction(name = "do_env_install_path_py")]
#[pyo3(
    signature = (env_path, iri, location),
)]
fn do_env_install_path_py(env_path: String, iri: String, location: String) -> PyResult<()> {
    common_init();

    let location: Utf8PathBuf = location.into();

    let mut env = LocalDirectoryEnvironment::read(env_path).map_err(env_read_to_pyerr)?;

    let metadata =
        wrapfs::metadata(&location).map_err(|e| PyErr::new::<PyIOError, _>(format_err(e)))?;
    if metadata.is_file() {
        let project = LocalKParProject::new_access(&location, KparInnerPath::Guess, None);

        let Some(version) = project
            .version()
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
        else {
            return Err(PyRuntimeError::new_err(format!(
                "project at `{location}` lacks project information"
            )));
        };

        let checksum = project
            .checksum_canonical_variant()
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?;
        env.put_project(iri, version, Some(checksum), |to| {
            clone_project(&project, to, true).map(|_| ())
        })
        .map_err(|e| PyRuntimeError::new_err(format_err(e)))?;
    } else if metadata.is_dir() {
        let project = LocalSrcProject::new_access(location, None);

        let Some(version) = project
            .version()
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?
        else {
            return Err(PyRuntimeError::new_err(format!(
                "project at {} lacks project information",
                project.root_path()
            )));
        };
        let checksum = project
            .checksum_canonical_variant()
            .map_err(|e| PyRuntimeError::new_err(format_err(e)))?;

        env.put_project(iri, version, Some(checksum), |to| {
            clone_project(&project, to, true).map(|_| ())
        })
        .map_err(|e| PyRuntimeError::new_err(format_err(e)))?;
    } else {
        return Err(PyRuntimeError::new_err(format!(
            "unable to find project at `{location}`"
        )));
    }

    Ok(())
}

#[pymodule(name = "_sysand_core")]
pub fn sysand_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    m.add_function(wrap_pyfunction!(do_init_py_local_file, m)?)?;
    m.add_function(wrap_pyfunction!(do_env_py_local_dir, m)?)?;
    m.add_function(wrap_pyfunction!(do_info_py_path, m)?)?;
    m.add_function(wrap_pyfunction!(do_model_roundtrip_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_info_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_root_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_build_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_sources_env_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_sources_project_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_add_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_remove_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_include_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_exclude_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_env_install_path_py, m)?)?;
    // Currently this interop is done with strings instead
    // m.add_class::<KparCompressionMethod>()?;

    m.add("DEFAULT_ENV_NAME", DEFAULT_ENV_NAME)?;
    Ok(())
}

fn env_read_to_pyerr(err: EnvMetadataError) -> PyErr {
    PyIOError::new_err(format!(
        "failed to read environment metadata: {}",
        format_err(err)
    ))
}

// Test-only helper: converts the python dicts to the Rust model types and
// back, so tests can verify that the typed dicts in `_model.py` stay in sync
// with `core/src/model.rs`. It cannot be compiled out of
// release wheels because CI runs pytest against the wheels it ships.
#[pyfunction(name = "_do_model_roundtrip_py")]
#[pyo3(
    signature = (info, metadata),
)]
fn do_model_roundtrip_py(
    info: &Bound<'_, PyAny>,
    metadata: &Bound<'_, PyAny>,
) -> PyResult<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
    Ok((info.extract()?, metadata.extract()?))
}

// Break the build when core types gain, lose, or rename a field/variant,
// since the typed dicts in `_model.py` cannot catch that on their own and
// must be updated together with this match.
// If this breaks, look if types in other bindings also need to be updated.
#[expect(unused, clippy::single_match)]
fn info_and_metadata_fields_guard(
    info: InterchangeProjectInfoRaw,
    meta: InterchangeProjectMetadataRaw,
) {
    let InterchangeProjectInfoRaw {
        name,
        publisher,
        description,
        version,
        license,
        maintainer,
        website,
        topic,
        usage,
    } = info;

    for usage in usage {
        match usage {
            InterchangeProjectUsageRaw::Resource {
                resource,
                version_constraint,
            } => {}
            InterchangeProjectUsageRaw::Directory {
                dir,
                publisher,
                name,
            } => {}
            InterchangeProjectUsageRaw::KparPath {
                kpar_path,
                publisher,
                name,
            } => {}
        }
    }

    let InterchangeProjectMetadataRaw {
        index,
        created,
        metamodel,
        includes_derived,
        includes_implied,
        checksum,
    } = meta;

    match checksum {
        Some(c) => {
            for (path, cksum) in c {
                let InterchangeProjectChecksumRaw { value, algorithm } = cksum;
            }
        }
        None => (),
    }
}

fn common_init() {
    let _ignored_noncritical_error = pyo3_log::try_init();
}
