// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{iter, process::ExitCode, str::FromStr as _, sync::Arc};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use pyo3::{
    PyTypeInfo as _,
    exceptions::{PyFileExistsError, PyFileNotFoundError, PyIOError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyAny, PyDict, PyType},
};
use semver::{Version, VersionReq};
use sysand::{
    CliAuthPolicy, DEFAULT_INDEX_URL,
    cli::ResolutionOptions,
    commands::{
        lock::{CliLockError, resolve_lock},
        sync::{CliSyncError, command_sync},
    },
    get_env, get_or_create_env, standard_auth_policy,
};
use sysand_core::{
    add::do_add_guess,
    auth::{GlobMapResult, StandardHTTPAuthenticationBuilder},
    build::{KParBuildError, KparCompressionMethod, do_build_kpar},
    commands::{
        env::{EnvError, do_env_local_dir},
        init::do_init_local_file,
        lock::{DEFAULT_LOCKFILE_NAME, LockError, LockProjectError},
        sync::{SyncOutcome, SyncedProject},
    },
    config::{Config, local_fs::load_configs},
    context::ProjectContext,
    discover::{discover_project, discover_workspace},
    env::{
        DEFAULT_ENV_NAME, ReadEnvironment as _, WriteEnvironment as _,
        discovery::DiscoveryError,
        index::{HttpFetchError, IndexEnvironmentError},
        local_directory::{
            LocalDirectoryEnvironment, LocalReadError, LocalWriteError,
            metadata::{EnvMetadataError, EnvProject, EnvProjectChecksum},
        },
        utils::clone_project,
    },
    exclude::do_exclude,
    include::do_include,
    index_location::IndexLocation,
    info::{InfoError, InfoProjectError, do_info, do_info_project},
    init::InitError,
    lock::{Lock, Project as LockedProject},
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsage, InterchangeProjectUsageRaw,
    },
    project::{
        ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::{EditInfoError, LocalSrcError, LocalSrcProject},
        memory::InMemoryProject,
        utils::{Identifier, wrapfs},
    },
    remove::do_remove_guess,
    resolve::{
        ResolveRead,
        combined::CombinedResolverError,
        net_utils::create_reqwest_client,
        priority::PriorityError,
        standard::{StandardResolver, standard_resolver},
    },
    root::do_root,
    solve::pubgrub::SolveConflict,
    sources::{Dependencies, do_sources_local_src_project_no_deps, resolve_dependencies},
    stdlib::known_std_libs,
    symbols::Language,
    usage::{ConstraintChange, SetConstraintError, do_set_usage_constraint_local},
    utils::ProvidedProjects,
    utils::format_err,
    versions::do_versions,
};
use typed_path::Utf8UnixPathBuf;

#[pyfunction(name = "_run_cli")]
fn run_cli(py: Python<'_>, args: Vec<String>) -> bool {
    // The CLI can run for seconds and talk to the network; holding the GIL
    // would block every other Python thread meanwhile — including an HTTP
    // server the CLI is talking to (the Python test suite's mock index).
    py.detach(|| run_cli_blocking(args))
}

fn run_cli_blocking(args: Vec<String>) -> bool {
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

/// Pure configuration of an authentication policy, as `AuthPolicy._spec()`
/// emits it. The Rust policy is built per call, inside `py.detach`.
#[derive(FromPyObject, Debug)]
#[pyo3(from_item_all)]
struct AuthSpec {
    kind: String,
    #[pyo3(default)]
    keyring: bool,
    #[pyo3(default)]
    url_glob: Option<String>,
    #[pyo3(default)]
    secret: Option<String>,
    #[pyo3(default)]
    username: Option<String>,
    #[pyo3(default)]
    label: Option<String>,
}

impl Default for AuthSpec {
    fn default() -> Self {
        Self {
            kind: "none".to_owned(),
            keyring: false,
            url_glob: None,
            secret: None,
            username: None,
            label: None,
        }
    }
}

/// Pure configuration of index resolution, as `Resolution._spec()` emits
/// it; mirrors the CLI's `ResolutionOptions`.
#[derive(FromPyObject, Debug)]
#[pyo3(from_item_all)]
struct ResolutionSpec {
    #[pyo3(default)]
    index: Vec<String>,
    #[pyo3(default)]
    default_index: Vec<String>,
    #[pyo3(default)]
    no_index: bool,
    #[pyo3(default)]
    include_std: bool,
    use_config: bool,
}

/// Every `AuthPolicy` variant becomes the same concrete policy type as the
/// CLI's, so the resolver stack is monomorphised once. `none()` is an
/// empty credential map, which behaves as unauthenticated.
fn build_auth_policy(spec: &AuthSpec) -> PyResult<Arc<CliAuthPolicy>> {
    let policy = match spec.kind.as_str() {
        "none" => CliAuthPolicy::without_store(
            StandardHTTPAuthenticationBuilder::new()
                .build()
                .map_err(|e| PyAuthError::new_err(format_err(e)))?,
        ),
        "env" => standard_auth_policy(spec.keyring)
            .map_err(|e| PyAuthError::new_err(format!("{e:#}")))?,
        "bearer" | "basic" => {
            let missing = || PyValueError::new_err("incomplete AuthPolicy specification");
            let url_glob = spec.url_glob.as_deref().ok_or_else(missing)?;
            let secret = spec.secret.as_deref().ok_or_else(missing)?;
            let mut builder = StandardHTTPAuthenticationBuilder::new();
            if spec.kind == "bearer" {
                builder.add_bearer_auth(
                    url_glob,
                    secret,
                    spec.label.as_deref().unwrap_or("python"),
                );
            } else {
                let username = spec.username.as_deref().ok_or_else(missing)?;
                builder.add_basic_auth(url_glob, username, secret);
            }
            CliAuthPolicy::without_store(builder.build().map_err(|e| {
                PyValueError::new_err(format!("invalid URL glob `{url_glob}`: {e}"))
            })?)
        }
        other => {
            return Err(PyValueError::new_err(format!(
                "unknown AuthPolicy kind `{other}`"
            )));
        }
    };
    Ok(Arc::new(policy))
}

/// The index URLs a call resolves against: `None` means "no index" (the
/// `no_index` flag, or no `Resolution` at all), otherwise the CLI's merge of
/// explicit indexes, configuration files, and the default index.
fn index_locations(
    spec: Option<&ResolutionSpec>,
    project_root: Option<&Utf8Path>,
) -> PyResult<Option<Vec<IndexLocation>>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    if spec.no_index {
        return Ok(None);
    }
    let config = if spec.use_config {
        load_configs(project_root.unwrap_or_else(|| Utf8Path::new(".")))
            .map_err(|e| ProjectError::new_err(format_err(e)))?
    } else {
        Config::default()
    };
    let locations = config
        .index_urls(
            spec.index.clone(),
            vec![DEFAULT_INDEX_URL.to_owned()],
            spec.default_index.clone(),
        )
        .map_err(|e| PyValueError::new_err(format_err(e)))?;
    Ok(Some(locations))
}

type StandardResolverError = <StandardResolver<CliAuthPolicy> as ResolveRead>::Error;

/// What to tell the user about an HTTP 401/403 from `url`, given the
/// policy that was in force. Never includes a secret.
fn auth_hint(auth: &AuthSpec, policy: &CliAuthPolicy, url: &str) -> String {
    match auth.kind.as_str() {
        "none" => format!(
            "no credentials were configured for `{url}`;\n\
            pass `auth=AuthPolicy.bearer(...)`, `AuthPolicy.basic(...)` or `AuthPolicy.from_env()`"
        ),
        "bearer" | "basic" => {
            let glob = auth.url_glob.as_deref().unwrap_or("");
            format!(
                "the {} credential for URL glob `{glob}`\n\
                was rejected by, or does not match, `{url}`",
                auth.kind
            )
        }
        _ => match policy.env_policy().publish_bearer_auth_map() {
            Ok(map) => match map.lookup(url) {
                GlobMapResult::Found(entry) => format!(
                    "the bearer token in `SYSAND_CRED_{label}_BEARER_TOKEN` (URL glob\n\
                     `SYSAND_CRED_{label}`) was rejected by `{url}`",
                    label = entry.label
                ),
                GlobMapResult::NotFound => {
                    format!("no `SYSAND_CRED_*` bearer token matches `{url}`")
                }
                GlobMapResult::Ambiguous(entries) => format!(
                    "several `SYSAND_CRED_*` URL globs match `{url}`:\n{}",
                    entries
                        .iter()
                        .map(|(_, e)| format!("`SYSAND_CRED_{}`", e.label))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            },
            Err(_) => format!("credentials from `SYSAND_CRED_*` were rejected by `{url}`"),
        },
    }
}

/// Typed exception for a failed `info`. The resolver's error type is known
/// statically, so this matches variants rather than walking `source()`
/// (the `transparent` wrappers in between would collapse that chain).
fn info_error_to_pyerr(
    err: InfoError<StandardResolverError>,
    auth: &AuthSpec,
    policy: &CliAuthPolicy,
) -> PyErr {
    let message = format_err(&err);
    match &err {
        InfoError::NotFound { .. } => PyNotFoundError::new_err(message),
        InfoError::Resolution(inner) => combined_error_to_pyerr(inner, message, auth, policy),
        _ => PyResolutionError::new_err(message),
    }
}

fn combined_error_to_pyerr<F, L, R>(
    err: &CombinedResolverError<F, L, R, IndexEnvironmentError>,
    message: String,
    auth: &AuthSpec,
    policy: &CliAuthPolicy,
) -> PyErr {
    match err {
        CombinedResolverError::Index(
            IndexEnvironmentError::Discovery(DiscoveryError::Fetch(fetch))
            | IndexEnvironmentError::Fetch(fetch),
        ) => match fetch {
            HttpFetchError::BadHttpStatus { url, status }
                if matches!(status.as_u16(), 401 | 403) =>
            {
                PyAuthError::new_err(format!("{message}\n  {}", auth_hint(auth, policy, url)))
            }
            // Connection-level failures are not the index's fault.
            HttpFetchError::Request { .. } => PyResolutionError::new_err(message),
            _ => PyIndexProtocolError::new_err(message),
        },
        CombinedResolverError::Index(_) => PyIndexProtocolError::new_err(message),
        _ => PyResolutionError::new_err(message),
    }
}

/// The resolver stack every index-reaching call uses: HTTP client, a
/// current-thread runtime, the configured indexes and the authentication
/// policy. Must run inside `py.detach` (the stack is `!Send`).
fn standard_resolver_for(
    resolution: Option<&ResolutionSpec>,
    auth: &AuthSpec,
    project_root: Option<&Utf8Path>,
) -> PyResult<(StandardResolver<CliAuthPolicy>, Arc<CliAuthPolicy>)> {
    let client = create_reqwest_client().map_err(|e| PyRuntimeError::new_err(format_err(e)))?;
    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    );
    let index_urls = index_locations(resolution, project_root)?;
    let auth_policy = build_auth_policy(auth)?;
    let resolver = standard_resolver(None, Some(client), index_urls, runtime, auth_policy.clone())
        .map_err(|err| PyValueError::new_err(format_err(err)))?;
    Ok((resolver, auth_policy))
}

fn parse_iri(iri: String) -> PyResult<Iri<String>> {
    Iri::parse(iri)
        .map_err(|(e, input)| PyValueError::new_err(format!("invalid IRI `{input}`: {e}")))
}

#[pyfunction(name = "do_info_py")]
#[pyo3(
    signature = (uri, resolution, auth),
)]
fn do_info_py(
    py: Python,
    uri: String,
    resolution: Option<ResolutionSpec>,
    auth: Option<AuthSpec>,
) -> PyResult<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
    common_init();

    py.detach(|| {
        let auth = auth.unwrap_or_default();
        // Without a `Resolution` no index is consulted
        let (resolver, auth_policy) = standard_resolver_for(resolution.as_ref(), &auth, None)?;
        let uri = parse_iri(uri)?;
        do_info(&uri, &resolver).map_err(|e| info_error_to_pyerr(e, &auth, &auth_policy))
    })
}

/// `(iri, versions highest first, ignored non-semver strings)`.
#[pyfunction(name = "do_versions_py")]
#[pyo3(
    signature = (iri, resolution, auth),
)]
fn do_versions_py(
    py: Python,
    iri: String,
    resolution: Option<ResolutionSpec>,
    auth: Option<AuthSpec>,
) -> PyResult<(String, Vec<String>, Vec<String>)> {
    common_init();

    py.detach(|| {
        let auth = auth.unwrap_or_default();
        let (resolver, auth_policy) = standard_resolver_for(resolution.as_ref(), &auth, None)?;
        let iri = parse_iri(iri)?;
        let listing = do_versions(&iri, &resolver)
            .map_err(|e| info_error_to_pyerr(e, &auth, &auth_policy))?;
        Ok((
            listing.iri,
            listing.versions.iter().map(Version::to_string).collect(),
            listing.ignored,
        ))
    })
}

/// A project the caller provides itself, as `ProvidedProject` dicts arrive.
#[derive(FromPyObject)]
#[pyo3(from_item_all)]
struct ProvidedSpec {
    iri: String,
    info: InterchangeProjectInfoRaw,
    meta: InterchangeProjectMetadataRaw,
}

fn provided_projects(specs: Vec<ProvidedSpec>) -> PyResult<ProvidedProjects> {
    let mut provided = ProvidedProjects::default();
    for spec in specs {
        let iri = parse_iri(spec.iri)?;
        provided
            .entry(Identifier::from_iri_owned(iri))
            .or_default()
            .push(InMemoryProject::from_info_meta(spec.info, spec.meta));
    }
    Ok(provided)
}

/// The CLI's view of where a call runs: the enclosing project and
/// workspace found from `start` and the environment that belongs to them
/// (`run_cli`'s own steps). Fails when `start` is not inside a project.
fn project_context(start: &Utf8Path) -> PyResult<(ProjectContext, Utf8PathBuf)> {
    let project_error = |e: String| ProjectError::new_err(e);
    let cwd = wrapfs::canonicalize(start).map_err(|e| project_error(format_err(e)))?;
    let current_project = discover_project(&cwd).map_err(|e| project_error(format_err(e)))?;
    let current_workspace = discover_workspace(&cwd).map_err(|e| project_error(format_err(e)))?;
    let Some(project) = &current_project else {
        return Err(project_error(format!(
            "`{cwd}` is not inside a project - neither it nor any of its parent directories \
             contain a SysML v2 or KerML project"
        )));
    };
    let project_root = project.root_path().to_owned();
    let env_root = current_workspace
        .as_ref()
        .map_or_else(|| project_root.clone(), |w| w.root_path().to_owned());
    let env = get_env(&env_root).map_err(|e| project_error(format!("{e:#}")))?;
    Ok((
        ProjectContext {
            env,
            current_workspace,
            current_project,
            current_directory: cwd,
        },
        project_root,
    ))
}

fn resolution_options(spec: &ResolutionSpec) -> ResolutionOptions {
    ResolutionOptions {
        index: spec.index.clone(),
        default_index: spec.default_index.clone(),
        no_index: spec.no_index,
        include_std: spec.include_std,
    }
}

fn config_for(spec: &ResolutionSpec, project_root: &Utf8Path) -> PyResult<Config> {
    if spec.use_config {
        load_configs(project_root).map_err(|e| ProjectError::new_err(format_err(e)))
    } else {
        Ok(Config::default())
    }
}

/// A failure computed without the GIL, turned into a Python exception once
/// the thread is attached again (the typed classes take keyword arguments
/// that `new_err` cannot pass).
enum Failure {
    Py(PyErr),
    Solve {
        message: String,
        report: String,
        kind: &'static str,
        conflicts: Vec<SolveConflict>,
    },
    /// The lockfile write itself failed after a successful solve.
    Wrote(String),
    /// `sync` failed part-way; `partial` is what it had done.
    Sync {
        message: String,
        wrote: bool,
        partial: SyncOutcome,
    },
    /// The environment could not be read or written.
    Env {
        message: String,
        wrote: bool,
    },
}

impl From<PyErr> for Failure {
    fn from(err: PyErr) -> Self {
        Self::Py(err)
    }
}

fn raise_with_kwargs(
    class: &Bound<'_, PyType>,
    message: String,
    kwargs: &Bound<'_, PyDict>,
) -> PyErr {
    match class.call((message,), Some(kwargs)) {
        Ok(exception) => PyErr::from_value(exception),
        Err(err) => err,
    }
}

fn conflict_to_dict<'py>(
    py: Python<'py>,
    conflict: &SolveConflict,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    match conflict {
        SolveConflict::Constraint {
            iri,
            constraint,
            required_by,
        } => {
            dict.set_item("kind", "Constraint")?;
            dict.set_item("iri", iri)?;
            dict.set_item("constraint", constraint)?;
            dict.set_item("required_by", required_by)?;
        }
        SolveConflict::NoVersions {
            iri,
            constraint,
            found,
            required_by,
        } => {
            dict.set_item("kind", "NoVersions")?;
            dict.set_item("iri", iri)?;
            dict.set_item("constraint", constraint)?;
            dict.set_item("found", found)?;
            dict.set_item("required_by", required_by)?;
        }
        SolveConflict::NotFound { iri, reason } => {
            dict.set_item("kind", "NotFound")?;
            dict.set_item("iri", iri)?;
            dict.set_item("reason", reason)?;
        }
    }
    Ok(dict)
}

impl Failure {
    fn into_pyerr(self, py: Python<'_>) -> PyErr {
        match self {
            Self::Py(err) => err,
            Self::Solve {
                message,
                report,
                kind,
                conflicts,
            } => {
                let kwargs = PyDict::new(py);
                let conflicts: PyResult<Vec<_>> =
                    conflicts.iter().map(|c| conflict_to_dict(py, c)).collect();
                let result = conflicts
                    .and_then(|conflicts| kwargs.set_item("conflicts", conflicts))
                    .and_then(|()| kwargs.set_item("report", report))
                    .and_then(|()| kwargs.set_item("kind", kind));
                match result {
                    Ok(()) => raise_with_kwargs(&PySolveError::type_object(py), message, &kwargs),
                    Err(err) => err,
                }
            }
            Self::Wrote(message) => {
                let kwargs = PyDict::new(py);
                match kwargs.set_item("wrote", true) {
                    Ok(()) => raise_with_kwargs(&ProjectError::type_object(py), message, &kwargs),
                    Err(err) => err,
                }
            }
            Self::Sync {
                message,
                wrote,
                partial,
            } => {
                let kwargs = PyDict::new(py);
                let result = outcome_dict(py, &partial)
                    .and_then(|partial| kwargs.set_item("partial", partial))
                    .and_then(|()| kwargs.set_item("wrote", wrote));
                match result {
                    Ok(()) => raise_with_kwargs(&PySyncError::type_object(py), message, &kwargs),
                    Err(err) => err,
                }
            }
            Self::Env { message, wrote } => {
                let kwargs = PyDict::new(py);
                match kwargs.set_item("wrote", wrote) {
                    Ok(()) => raise_with_kwargs(&PyEnvError::type_object(py), message, &kwargs),
                    Err(err) => err,
                }
            }
        }
    }
}

/// Typed failure for `resolve_lock`'s `anyhow::Error`: solver failures
/// become `SolveError` with their conflicts, unless the solver itself hit a
/// transport or authentication problem, which is classified like `info`'s.
fn lock_error_to_failure(err: anyhow::Error, auth: &AuthSpec, policy: &CliAuthPolicy) -> Failure {
    let message = format!("{err:#}");
    match err.downcast_ref::<CliLockError<CliAuthPolicy>>() {
        Some(LockProjectError::LockError(LockError::Solver(solver))) => {
            if let Some(PriorityError::Lower(inner)) = solver.resolution_error() {
                return Failure::Py(combined_error_to_pyerr(inner, message, auth, policy));
            }
            Failure::Solve {
                message,
                report: solver.to_string(),
                kind: solver.kind(),
                conflicts: solver.conflicts(),
            }
        }
        Some(_) | None => Failure::Py(ProjectError::new_err(message)),
    }
}

/// Returns `(canonical lockfile text, projects)`
#[pyfunction(name = "do_lock_py")]
#[pyo3(
    signature = (path, resolution, auth, provided, write),
)]
fn do_lock_py(
    py: Python,
    path: String,
    resolution: ResolutionSpec,
    auth: Option<AuthSpec>,
    provided: Vec<ProvidedSpec>,
    write: bool,
) -> PyResult<(String, Vec<LockedProject>)> {
    common_init();

    let provided = provided_projects(provided)?;
    let outcome: Result<(String, Vec<LockedProject>), Failure> = py.detach(|| {
        let auth = auth.unwrap_or_default();
        let (ctx, project_root) = project_context(Utf8Path::new(&path))?;
        let config = config_for(&resolution, &project_root)?;
        let client = create_reqwest_client().map_err(|e| PyRuntimeError::new_err(format_err(e)))?;
        let runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(PyErr::from)?,
        );
        let auth_policy = build_auth_policy(&auth)?;

        let lock: Lock = resolve_lock(
            ".",
            resolution_options(&resolution),
            &config,
            &project_root,
            provided,
            client,
            runtime,
            auth_policy.clone(),
            &ctx,
        )
        .map_err(|e| lock_error_to_failure(e, &auth, &auth_policy))?;

        let text = lock.to_string();
        if write {
            wrapfs::write(project_root.join(DEFAULT_LOCKFILE_NAME), &text)
                .map_err(|e| Failure::Wrote(format_err(e)))?;
        }
        Ok((text, lock.projects))
    });
    outcome.map_err(|failure| failure.into_pyerr(py))
}

/// `(iri, version, install path)` per synced project; the path is
/// environment-relative and `None` once pruned.
type SyncedTuple = (String, String, Option<String>);

fn synced_tuples(
    entries: &[SyncedProject],
    env: Option<&LocalDirectoryEnvironment>,
) -> Vec<SyncedTuple> {
    entries
        .iter()
        .map(|entry| {
            let path = env.and_then(|env| {
                env.projects()
                    .iter()
                    .find(|p| p.version == entry.version && p.identifiers.contains(&entry.iri))
                    .map(|p| p.path.as_str().to_owned())
            });
            (entry.iri.clone(), entry.version.clone(), path)
        })
        .collect()
}

/// A `SyncOutcome` as the `SyncOutcome` typed dict, for `SyncError.partial`.
/// Install paths are not resolved here: the environment metadata is not
/// rewritten after a failed sync.
fn outcome_dict<'py>(py: Python<'py>, outcome: &SyncOutcome) -> PyResult<Bound<'py, PyDict>> {
    let entries = |entries: &[SyncedProject]| -> PyResult<Vec<Bound<'py, PyDict>>> {
        entries
            .iter()
            .map(|entry| {
                let dict = PyDict::new(py);
                dict.set_item("iri", &entry.iri)?;
                dict.set_item("version", &entry.version)?;
                dict.set_item("path", py.None())?;
                Ok(dict)
            })
            .collect()
    };
    let dict = PyDict::new(py);
    dict.set_item("installed", entries(&outcome.installed)?)?;
    dict.set_item("pruned", entries(&outcome.pruned)?)?;
    dict.set_item("kept", entries(&outcome.kept)?)?;
    Ok(dict)
}

/// Typed failure for `command_sync`'s `anyhow::Error`, with what had been
/// done before it.
fn sync_error_to_failure(err: anyhow::Error, outcome: SyncOutcome) -> Failure {
    let message = format!("{err:#}");
    let wrote = outcome.wrote();
    match err.downcast_ref::<CliSyncError>() {
        Some(CliSyncError::EnvRead(_) | CliSyncError::EnvWrite(_)) => {
            Failure::Env { message, wrote }
        }
        Some(_) => Failure::Sync {
            message,
            wrote,
            partial: outcome,
        },
        // `merge_lock` + `write()` of the environment metadata, or anything
        // else outside the sync loop.
        None => Failure::Env { message, wrote },
    }
}

/// `(installed, pruned, kept)` as `(iri, version, path)` tuples.
#[pyfunction(name = "do_sync_py")]
#[pyo3(
    signature = (path, lock_text, resolution, auth, provided, no_prune),
)]
fn do_sync_py(
    py: Python,
    path: String,
    lock_text: Option<String>,
    resolution: ResolutionSpec,
    auth: Option<AuthSpec>,
    provided: Vec<ProvidedSpec>,
    no_prune: bool,
) -> PyResult<(Vec<SyncedTuple>, Vec<SyncedTuple>, Vec<SyncedTuple>)> {
    common_init();

    let extra_provided = provided_projects(provided)?;
    let outcome: Result<_, Failure> = py.detach(|| {
        let auth = auth.unwrap_or_default();
        let (mut ctx, project_root) = project_context(Utf8Path::new(&path))?;

        // The lockfile is read before the environment is created, so a
        // missing lockfile leaves nothing behind. No implicit `lock`.
        let lockfile_path = project_root.join(DEFAULT_LOCKFILE_NAME);
        let lock_text = match lock_text {
            Some(text) => text,
            None => wrapfs::read_to_string(&lockfile_path).map_err(|e| {
                ProjectError::new_err(format!(
                    "no lockfile at `{lockfile_path}`; run `lock` first ({})",
                    format_err(e)
                ))
            })?,
        };
        let lock = Lock::from_str(&lock_text)
            .map_err(|e| ProjectError::new_err(format!("invalid lockfile: {}", format_err(e))))?;

        let mut provided = if resolution.include_std {
            ProvidedProjects::default()
        } else {
            known_std_libs()
        };
        provided.extend(extra_provided);

        let mut env = get_or_create_env(
            ctx.env.take(),
            ctx.current_workspace.as_ref(),
            ctx.current_project.as_ref(),
            &ctx.current_directory,
        )
        .map_err(|e| Failure::Env {
            message: format!("{e:#}"),
            wrote: false,
        })?;

        let client = create_reqwest_client().map_err(|e| PyRuntimeError::new_err(format_err(e)))?;
        let runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(PyErr::from)?,
        );
        let auth_policy = build_auth_policy(&auth)?;

        let mut outcome = SyncOutcome::default();
        command_sync(
            &lock,
            &project_root,
            &mut env,
            client,
            &provided,
            runtime,
            auth_policy,
            ctx.current_workspace.as_ref(),
            no_prune,
            &mut outcome,
        )
        .map_err(|e| sync_error_to_failure(e, outcome.clone()))?;

        Ok((
            synced_tuples(&outcome.installed, Some(&env)),
            synced_tuples(&outcome.pruned, None),
            synced_tuples(&outcome.kept, Some(&env)),
        ))
    });
    outcome.map_err(|failure| failure.into_pyerr(py))
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
fn do_add_py(path: String, iri: String, version: Option<String>) -> PyResult<bool> {
    common_init();

    let mut project = LocalSrcProject::new_access(path, None);

    // TODO: do dependency resolution and locking?
    // `true` when a new usage was added, `false` when it was merged into an
    // existing one.
    do_add_guess(&mut project, iri, version).map_err(|e| PyRuntimeError::new_err(format_err(e)))
}

/// The exception classes live in Python (`sysand/_errors.py`) so that `mypy`
/// sees their attributes; the Rust side only needs to raise them.
mod py_errors {
    // `import_exception!` generates an inherent `new_err` next to the
    // `PyTypeInfo` trait method of the same name.
    #![allow(clippy::same_name_method)]
    pyo3::import_exception!(sysand._errors, ProjectError);
    pyo3::import_exception!(sysand._errors, EnvError);
    pyo3::import_exception!(sysand._errors, ResolutionError);
    pyo3::import_exception!(sysand._errors, NotFoundError);
    pyo3::import_exception!(sysand._errors, AuthError);
    pyo3::import_exception!(sysand._errors, IndexProtocolError);
    pyo3::import_exception!(sysand._errors, SolveError);
    pyo3::import_exception!(sysand._errors, SyncError);
}
// `sysand_core::commands::env::EnvError` is already in scope under that name.
use py_errors::{
    AuthError as PyAuthError, EnvError as PyEnvError, IndexProtocolError as PyIndexProtocolError,
    NotFoundError as PyNotFoundError, ProjectError, ResolutionError as PyResolutionError,
    SolveError as PySolveError, SyncError as PySyncError,
};

/// Returns `(matched_resource, found, changed, old_constraint, new_constraint)`.
///
/// Invalid input (a malformed shorthand or an invalid version requirement)
/// raises `ValueError`; a missing, unreadable or malformed manifest and an
/// ambiguous declaration raise `sysand.ProjectError` (`wrote` stays `False`:
/// nothing is written unless the edit succeeds). A missing usage is *not* an
/// error here — `found` is `False` and the Python side decides.
#[pyfunction(name = "do_set_usage_constraint_py")]
#[pyo3(
    signature = (path, resource, constraint),
)]
fn do_set_usage_constraint_py(
    path: String,
    resource: String,
    constraint: Option<String>,
) -> PyResult<(String, bool, bool, Option<String>, Option<String>)> {
    common_init();

    let mut project = LocalSrcProject::new_access(path, None);

    match do_set_usage_constraint_local(&mut project, &resource, constraint.as_deref()) {
        Ok((resource, ConstraintChange::Replaced { old, new })) => {
            Ok((resource, true, true, old, new))
        }
        Ok((resource, ConstraintChange::Unchanged { constraint })) => {
            Ok((resource, true, false, constraint.clone(), constraint))
        }
        Ok((resource, ConstraintChange::NotFound)) => Ok((resource, false, false, None, None)),
        Err(EditInfoError::Edit(
            e @ (SetConstraintError::InvalidConstraint(..) | SetConstraintError::MalformedUsage(_)),
        )) => Err(PyValueError::new_err(format_err(e))),
        Err(e) => Err(ProjectError::new_err(format_err(e))),
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
    m.add_function(wrap_pyfunction!(do_versions_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_lock_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_sync_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_root_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_build_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_sources_env_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_sources_project_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_add_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_set_usage_constraint_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_remove_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_include_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_exclude_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_env_install_path_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_env_projects_py, m)?)?;
    m.add_function(wrap_pyfunction!(do_discover_py, m)?)?;
    // Currently this interop is done with strings instead
    // m.add_class::<KparCompressionMethod>()?;

    m.add("DEFAULT_ENV_NAME", DEFAULT_ENV_NAME)?;
    Ok(())
}

fn env_read_to_pyerr(err: EnvMetadataError) -> PyErr {
    PyEnvError::new_err(format_err(err))
}

/// The entries of the environment's `env.toml`, each converted to a dict by
/// `EnvProject`'s `IntoPyObject`. `path` is returned verbatim (relative to the
/// environment directory, or to the workspace/project root for editable
/// entries).
#[pyfunction(name = "do_env_projects_py")]
#[pyo3(
    signature = (env_path),
)]
fn do_env_projects_py(env_path: String) -> PyResult<Vec<EnvProject>> {
    common_init();

    let env = LocalDirectoryEnvironment::read(env_path).map_err(env_read_to_pyerr)?;
    Ok(env.projects().to_vec())
}

/// `(project_root, workspace_root)`, both canonicalized as `do_root_py` does,
/// found by walking up from `path` exactly as the CLI does before every
/// command.
#[pyfunction(name = "do_discover_py")]
#[pyo3(
    signature = (path),
)]
fn do_discover_py(path: String) -> PyResult<(Option<String>, Option<String>)> {
    common_init();

    let path = Utf8PathBuf::from(path);
    let project_root = discover_project(&path)
        .map_err(|e| ProjectError::new_err(format_err(e)))?
        .map(|project| wrapfs::canonicalize(project.root_path()))
        .transpose()
        .map_err(|e| ProjectError::new_err(format_err(e)))?;
    let workspace_root = discover_workspace(&path)
        .map_err(|e| ProjectError::new_err(format_err(e)))?
        .map(|workspace| wrapfs::canonicalize(workspace.root_path()))
        .transpose()
        .map_err(|e| ProjectError::new_err(format_err(e)))?;
    Ok((
        project_root.map(Utf8PathBuf::into_string),
        workspace_root.map(Utf8PathBuf::into_string),
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

// Same purpose as `info_and_metadata_fields_guard`, for the `EnvProject`
// typed dict in `_model.py`.
#[expect(unused)]
fn env_project_fields_guard(project: EnvProject) {
    let EnvProject {
        publisher,
        name,
        version,
        path,
        identifiers,
        usages,
        editable,
        workspace,
        checksum,
    } = project;

    match checksum {
        Some(EnvProjectChecksum::Kpar { kpar_cksum }) => {}
        Some(EnvProjectChecksum::Project { src_cksum }) => {}
        None => (),
    }
}

fn common_init() {
    let _ignored_noncritical_error = pyo3_log::try_init();
}
