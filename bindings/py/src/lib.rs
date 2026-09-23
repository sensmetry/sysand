// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{iter, str::FromStr as _, sync::Arc};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use pyo3::{
    exceptions::{PyFileExistsError, PyFileNotFoundError, PyIOError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyAny, PyDict},
};
use semver::{Version, VersionReq};
use sysand::{
    CliAuthPolicy, DEFAULT_INDEX_URL,
    cli::ResolutionOptions,
    commands::{
        lock::{CliLockError, resolve_lock},
        sync::{CliSyncError, CommandSyncError, command_sync},
    },
    get_env, get_or_create_env, standard_auth_policy,
};
use sysand_core::{
    add::{AddError, do_add},
    auth::{GlobMapResult, StandardHTTPAuthenticationBuilder},
    build::{KParBuildError, KparCompressionMethod, do_build_kpar},
    commands::{
        env::{EnvError, do_env_local_dir},
        init::do_init_local_file,
        lock::{DEFAULT_LOCKFILE_NAME, LockError, LockProjectError},
        sync::SyncOutcome,
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
        local_src::{LocalSrcError, LocalSrcProject},
        memory::InMemoryProject,
        utils::{Identifier, wrapfs},
    },
    purl::{is_valid_unnormalized_name, is_valid_unnormalized_publisher},
    remove::{RemoveError, do_remove},
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
    usage::{ConstraintChange, do_set_usage_constraint_local},
    utils::ProvidedProjects,
    utils::format_err,
    versions::do_versions,
};
use typed_path::Utf8UnixPathBuf;

mod model;
use model::{PyInfo, PyUsage, index_purl};

#[pyfunction(name = "_run_cli")]
fn run_cli(py: Python<'_>, args: Vec<String>) -> u8 {
    // The CLI can run for seconds and talk to the network; holding the GIL
    // would block every other Python thread meanwhile — including an HTTP
    // server the CLI is talking to (the Python test suite's mock index).
    py.detach(|| run_cli_blocking(args))
}

fn run_cli_blocking(args: Vec<String>) -> u8 {
    // CMD and PowerShell leave glob patterns to the program
    sysand::lib_main_with(
        sysand::expand_globs(args),
        sysand::ProcessOwnership::Embedded,
    )
}

/// Clap's long help for the command `args` names, as a string.
///
/// For an embedding CLI that must *return* help text rather than let it be
/// printed: clap writes to the process's file descriptors, which a Python
/// stream redirect of `sys.stdout`/`sys.stderr` does not capture. `prog` is the
/// program name to spell in usage lines.
///
/// Unparseable arguments give clap's error text, not an exception: a caller
/// asking for help has nowhere useful to put a failure.
///
/// No `py.detach`, unlike `_run_cli`: this is string formatting, with no I/O
/// and no network.
#[pyfunction(name = "_render_long_help")]
#[pyo3(
    signature = (prog, args),
)]
fn render_long_help(prog: &str, args: Vec<String>) -> String {
    sysand::render_long_help(prog, args)
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
    // library from python runs it -- and an embedder whose CLI is a Python
    // entry point imports this module before it can reach `_run_cli` at all,
    // so at import the CLI would never get its own logger. When the order
    // does go the other way, `_run_cli` passes `ProcessOwnership::Embedded`
    // and the CLI keeps the level without complaining about the formatting.
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
fn do_info_py_path(path: String) -> PyResult<(PyInfo, InterchangeProjectMetadataRaw)> {
    common_init();

    let project = LocalSrcProject::new_access(path, None);

    match do_info_project(&project) {
        Ok((info, meta)) => Ok((info.into(), meta)),
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
) -> PyResult<(PyInfo, InterchangeProjectMetadataRaw)> {
    common_init();

    py.detach(|| {
        let auth = auth.unwrap_or_default();
        // Without a `Resolution` no index is consulted
        let (resolver, auth_policy) = standard_resolver_for(resolution.as_ref(), &auth, None)?;
        let uri = parse_iri(uri)?;
        do_info(&uri, &resolver)
            .map(|(info, meta)| (info.into(), meta))
            .map_err(|e| info_error_to_pyerr(e, &auth, &auth_policy))
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
    info: PyInfo,
    meta: InterchangeProjectMetadataRaw,
}

fn provided_projects(specs: Vec<ProvidedSpec>) -> PyResult<ProvidedProjects> {
    let mut provided = ProvidedProjects::default();
    for spec in specs {
        let iri = parse_iri(spec.iri)?;
        provided
            .entry(Identifier::from_iri_owned(iri))
            .or_default()
            .push(InMemoryProject::from_info_meta(spec.info.into(), spec.meta));
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
            defaulted,
            found,
            required_by,
        } => {
            dict.set_item("kind", "NoVersions")?;
            dict.set_item("iri", iri)?;
            dict.set_item("constraint", constraint)?;
            dict.set_item("defaulted", defaulted)?;
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
                    Ok(()) => PySolveError::new_err_with(py, message, &kwargs),
                    Err(err) => err,
                }
            }
            Self::Wrote(message) => {
                let kwargs = PyDict::new(py);
                match kwargs.set_item("wrote", true) {
                    Ok(()) => ProjectError::new_err_with(py, message, &kwargs),
                    Err(err) => err,
                }
            }
            Self::Sync {
                message,
                wrote,
                partial,
            } => {
                let kwargs = PyDict::new(py);
                let result = kwargs
                    .set_item("partial", partial)
                    .and_then(|()| kwargs.set_item("wrote", wrote));
                match result {
                    Ok(()) => PySyncError::new_err_with(py, message, &kwargs),
                    Err(err) => err,
                }
            }
            Self::Env { message, wrote } => {
                let kwargs = PyDict::new(py);
                match kwargs.set_item("wrote", wrote) {
                    Ok(()) => PyEnvError::new_err_with(py, message, &kwargs),
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

/// `command_sync`'s failure as the exception to raise, with what had been
/// done before it. Environment failures, including writing its metadata
/// after the sync loop, are `EnvError`s; anything else is a `SyncError`.
fn sync_failure(err: CommandSyncError, outcome: SyncOutcome) -> Failure {
    let message = format_err(&err);
    let wrote = outcome.wrote();
    match err {
        CommandSyncError::Sync(CliSyncError::EnvRead(_) | CliSyncError::EnvWrite(_))
        | CommandSyncError::WriteMetadata(_) => Failure::Env { message, wrote },
        CommandSyncError::Sync(_) => Failure::Sync {
            message,
            wrote,
            partial: outcome,
        },
    }
}

/// The outcome as a `SyncOutcome` dict, and the environment's entries once
/// it is written, from which the Python side takes each synced project's
/// install path.
#[pyfunction(name = "do_sync_py")]
#[pyo3(
    signature = (path, lock_text, resolution, auth, provided),
)]
fn do_sync_py(
    py: Python,
    path: String,
    lock_text: Option<String>,
    resolution: ResolutionSpec,
    auth: Option<AuthSpec>,
    provided: Vec<ProvidedSpec>,
) -> PyResult<(SyncOutcome, Vec<EnvProject>)> {
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
        if let Err(err) = command_sync(
            &lock,
            &project_root,
            &mut env,
            client,
            &provided,
            runtime,
            auth_policy,
            ctx.current_workspace.as_ref(),
            // Always prune: `no_prune` is not exposed.
            false,
            &mut outcome,
        ) {
            return Err(sync_failure(err, outcome));
        }
        Ok((outcome, env.projects().to_vec()))
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

/// Adds a resource usage of `iri`, taken literally: the `publisher/name`
/// shorthand is not expanded, so anything but an IRI is refused.
#[pyfunction(name = "do_add_py")]
#[pyo3(
    signature = (path, iri, version_constraint),
)]
fn do_add_py(path: String, iri: String, version_constraint: String) -> PyResult<bool> {
    common_init();

    let mut project = LocalSrcProject::new_access(path, None);
    let usage = InterchangeProjectUsageRaw::Resource {
        resource: iri,
        version_constraint: Some(version_constraint),
    };

    // TODO: do dependency resolution and locking?
    // `true` when a new usage was added, `false` when it was merged into an
    // existing one.
    do_add(&mut project, &usage).map_err(|err| match err {
        // The core message says to remove the other usage first, which the
        // Python API cannot do for a directory or KPAR usage.
        AddError::DuplicateIdentifier {
            identifier,
            existing,
            new,
        } => ProjectError::new_err(format!(
            "`{identifier}` is already declared as a {existing} usage, so it cannot \
             also be added as a {new} usage; the Python API cannot remove \
             directory and KPAR usages yet"
        )),
        err => ProjectError::new_err(format_err(err)),
    })
}

/// The `pkg:sysand` PURL that `add`, `remove` and `set_usage_constraint`
/// name the index usage of `publisher` and `name` by. Either may be given
/// unnormalized (`Acme Labs`), so that the same values can later declare a
/// typed index usage, which keeps them as given; the PURL holds them
/// normalized (`acme-labs`).
#[pyfunction(name = "index_purl_py")]
fn index_purl_py(publisher: &str, name: &str) -> PyResult<String> {
    if !is_valid_unnormalized_publisher(publisher) {
        return Err(ProjectError::new_err(format!(
            "publisher `{publisher}` is not valid: it must be 3-50 characters,\n\
             use only ASCII letters and numbers, may include single spaces or\n\
             hyphens between words, and must start and end with a letter or number"
        )));
    }
    if !is_valid_unnormalized_name(name) {
        return Err(ProjectError::new_err(format!(
            "name `{name}` is not valid: it must be 3-50 characters, use only\n\
             ASCII letters and numbers, may include single spaces, hyphens, or\n\
             dots between words, and must start and end with a letter or number"
        )));
    }
    Ok(index_purl(publisher, name))
}

/// Refuses anything that is not an IRI a resource usage could name, with the
/// message `do_add_py` gives for it. `do_remove` and
/// `do_set_usage_constraint_local` take their argument as given, without
/// checking that it is an IRI, so a typo would otherwise be reported as
/// "not found".
fn validate_iri(iri: &str) -> PyResult<()> {
    InterchangeProjectUsageRaw::Resource {
        resource: iri.to_owned(),
        version_constraint: None,
    }
    .validate()
    .map(drop)
    .map_err(|e| ProjectError::new_err(format_err(e)))
}

/// The exception classes live in Python (`_errors.py`) so that `mypy` sees
/// their attributes; the Rust side only needs to raise them, and is handed
/// them by `_register_errors` rather than importing them by module path.
#[expect(
    dead_code,
    reason = "the macro gives every class both constructors, and most classes \
              are only ever raised one of the two ways"
)]
mod py_errors {
    use pyo3::{
        Bound, Py, PyErr, PyResult, Python,
        exceptions::PyRuntimeError,
        pyfunction,
        sync::PyOnceLock,
        types::{PyAnyMethods as _, PyDict, PyType},
    };

    /// Raised in place of the intended class when nothing registered one.
    ///
    /// Reachable only by importing this extension without the Python package
    /// that owns it, so the message names that missing import rather than
    /// leaving an embedder with a class it has never heard of.
    fn unregistered(message: &str) -> PyErr {
        PyRuntimeError::new_err(format!(
            "{message}\n\nnote: sysand's exception classes have not been\n\
             registered with this extension module. Import the Python\n\
             package that owns it before calling into the bindings."
        ))
    }

    macro_rules! registered_exceptions {
        ($($field:ident => $name:ident),+ $(,)?) => {
            struct ErrorTypes {
                $($field: Py<PyType>,)+
            }

            static ERROR_TYPES: PyOnceLock<ErrorTypes> = PyOnceLock::new();

            /// Hand this extension the exception classes it should raise.
            ///
            /// Called by `_errors.py` when the package is imported.
            ///
            /// The first registration wins. All subsequent ones are ignored, since
            /// the calling module may be initialized multiple times (e.g. `importlib.reload`)
            #[pyfunction(name = "_register_errors")]
            #[pyo3(signature = (*, $($field),+))]
            pub fn register_errors(
                py: Python<'_>,
                $($field: Py<PyType>,)+
            ) -> PyResult<()> {
                if ERROR_TYPES.set(py, ErrorTypes { $($field,)+ }).is_err() {
                    log::debug!(
                        "sysand's exception classes are already registered; \
                         keeping the ones registered first"
                    );
                }
                Ok(())
            }

            $(
                pub struct $name;

                impl $name {
                    /// Raise the registered class with `message` alone.
                    pub fn new_err(message: impl Into<String>) -> PyErr {
                        Python::attach(|py| Self::raise(py, message.into(), None))
                    }

                    /// Raise it with keyword arguments as well (`wrote`,
                    /// `conflicts`, `partial`, ...).
                    pub fn new_err_with<'py>(
                        py: Python<'py>,
                        message: String,
                        kwargs: &Bound<'py, PyDict>,
                    ) -> PyErr {
                        Self::raise(py, message, Some(kwargs))
                    }

                    fn raise<'py>(
                        py: Python<'py>,
                        message: String,
                        kwargs: Option<&Bound<'py, PyDict>>,
                    ) -> PyErr {
                        let Some(types) = ERROR_TYPES.get(py) else {
                            return unregistered(&message);
                        };
                        match types.$field.bind(py).call((message,), kwargs) {
                            Ok(exception) => PyErr::from_value(exception),
                            Err(err) => err,
                        }
                    }
                }
            )+
        };
    }

    registered_exceptions! {
        project => ProjectError,
        env => EnvError,
        resolution => ResolutionError,
        not_found => NotFoundError,
        auth => AuthError,
        index_protocol => IndexProtocolError,
        solve => SolveError,
        sync => SyncError,
    }
}
// `sysand_core::commands::env::EnvError` is already in scope under that name.
use py_errors::{
    AuthError as PyAuthError, EnvError as PyEnvError, IndexProtocolError as PyIndexProtocolError,
    NotFoundError as PyNotFoundError, ProjectError, ResolutionError as PyResolutionError,
    SolveError as PySolveError, SyncError as PySyncError, register_errors,
};

/// Returns `(found, changed, old_version_constraint, new_version_constraint)`.
///
/// Every failure raises `ProjectError` (`wrote` stays `False`: nothing is
/// written unless the edit succeeds), matching `do_add_py` and
/// `do_remove_py`, which reject a non-IRI or an invalid version requirement
/// the same way. A missing usage is *not* an error here —
/// `found` is `False` and the Python side decides.
#[pyfunction(name = "do_set_usage_constraint_py")]
#[pyo3(
    signature = (path, iri, version_constraint),
)]
fn do_set_usage_constraint_py(
    path: String,
    iri: String,
    version_constraint: String,
) -> PyResult<(bool, bool, Option<String>, Option<String>)> {
    common_init();

    validate_iri(&iri)?;
    let mut project = LocalSrcProject::new_access(path, None);

    match do_set_usage_constraint_local(&mut project, &iri, &version_constraint) {
        Ok(ConstraintChange::Replaced { old, new }) => Ok((true, true, old, Some(new))),
        Ok(ConstraintChange::Unchanged { constraint }) => {
            Ok((true, false, Some(constraint.clone()), Some(constraint)))
        }
        Ok(ConstraintChange::NotFound) => Ok((false, false, None, None)),
        Err(e) => Err(ProjectError::new_err(format_err(e))),
    }
}

/// Removes the resource usages of `iri`, taken literally, as `do_add_py`
/// takes it. Returns the usages that were removed, in declaration order.
#[pyfunction(name = "do_remove_py")]
#[pyo3(
    signature = (path, iri),
)]
fn do_remove_py(path: String, iri: String) -> PyResult<Vec<PyUsage>> {
    common_init();

    validate_iri(&iri)?;
    let mut project = LocalSrcProject::new_access(path, None);

    let removed = do_remove(&mut project, iri).map_err(|err| match err {
        // The core message points at a CLI command, which is no help here.
        RemoveError::UsageIsTyped { identifier, kind } => ProjectError::new_err(format!(
            "`{identifier}` is declared as a {kind} usage, not as a resource \
             or index usage; the Python API cannot remove directory and KPAR usages yet"
        )),
        err => ProjectError::new_err(format_err(err)),
    })?;
    Ok(removed.into_iter().map(PyUsage::from).collect())
}

/// `src_path` must be relative to and under the project root
/// and use Unix separators. No normalization will be performed
#[pyfunction(name = "do_include_py")]
#[pyo3(
    signature = (path, src_path, compute_checksum, index_symbols),
)]
fn do_include_py(
    path: String,
    src_path: String,
    compute_checksum: bool,
    index_symbols: bool,
) -> PyResult<()> {
    common_init();

    let mut project = LocalSrcProject::new_access(path, None);

    do_include(
        &mut project,
        iter::once(Utf8UnixPathBuf::from(src_path)),
        compute_checksum,
        index_symbols,
        None,
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
    m.add_function(wrap_pyfunction!(register_errors, m)?)?;
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    m.add_function(wrap_pyfunction!(render_long_help, m)?)?;
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
    m.add_function(wrap_pyfunction!(index_purl_py, m)?)?;
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
) -> PyResult<(PyInfo, InterchangeProjectMetadataRaw)> {
    // Through core's type, so a field the view drops shows up as a diff.
    let info: InterchangeProjectInfoRaw = info.extract::<PyInfo>()?.into();
    Ok((info.into(), metadata.extract()?))
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
