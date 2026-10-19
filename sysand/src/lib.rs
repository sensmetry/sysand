// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    iter,
    sync::{Arc, LazyLock},
    time::Duration,
};

use anstream::eprintln;
use anyhow::{Result, anyhow, bail};
use fluent_uri::Iri;

use camino::{Utf8Path, Utf8PathBuf};
use clap::{CommandFactory as _, Parser as _};
use sysand_core::{
    auth::{
        HTTPAuthentication, StandardHTTPAuthentication, StandardHTTPAuthenticationBuilder,
        StandardLazyHTTPAuthentication,
    },
    cancel::CancellationToken,
    commands::{lock::DEFAULT_LOCKFILE_NAME, sync::SyncOutcome, verify::ManifestUsages},
    config::{
        Config,
        local_fs::{CONFIG_FILE, get_config, load_configs, read_config},
    },
    context::ProjectContext,
    discover::discover_project,
    env::{
        DEFAULT_ENV_NAME,
        local_directory::{LocalDirectoryEnvironment, metadata::EnvMetadataError},
    },
    index::RemoveTarget,
    index_location::IndexSource,
    lock::Lockfile,
    manifest::{diagnostics::ReadWarning, workspace_table::WorkspaceTable},
    model::InterchangeProjectUsage,
    project::{
        any::{AnyProject, OverrideProject},
        local_src::LocalSrcProject,
        reference::ProjectReference,
        utils::{Identifier, wrapfs},
    },
    resolve::{ResolutionInfo, file::FileResolverProject, net_utils::create_reqwest_client},
    stdlib::known_std_libs,
    workspace::model_toml::{
        DiscoverError, MemberChecks, Selector, StartRegion, Workspace, discover_table,
        member_index, start_region,
    },
    workspace_lock::{ExclusiveLock, LockMode, WorkspaceLock},
};
use url::Url;

use crate::{
    cli::{Args, AuthCommand, Command, EnvCommand, IndexCommand},
    commands::{
        add::command_add,
        auth::{command_auth_login, command_auth_logout, command_auth_status, command_auth_whoami},
        build::{command_build_for_project, command_build_for_workspace},
        clone::command_clone,
        edit::command_edit,
        env::{command_env, command_env_list},
        exclude::command_exclude,
        include::command_include,
        index::{command_index_add, command_index_init, command_index_remove, command_index_yank},
        info::{
            LocalProjectKind, command_info_field, command_info_field_path, command_info_path,
            command_info_project,
        },
        init::{command_init, parse_metamodel},
        lock::{command_lock, command_lock_workspace},
        print_root::command_print_root,
        publish::command_publish,
        remove::command_remove,
        sources::{command_sources_env, command_sources_project, command_sources_workspace},
        sync::{VerifyStatus, command_sync, command_sync_verify},
    },
    output::RunReport,
};

const DEFAULT_INDEX_URL: &str = "https://sysand.com";

/// [`DEFAULT_INDEX_URL`], parsed once
pub fn default_index_location() -> IndexSource {
    static LOCATION: LazyLock<IndexSource> = LazyLock::new(|| {
        IndexSource::parse(DEFAULT_INDEX_URL).expect("BUG: DEFAULT_INDEX_URL is a valid index URL")
    });
    LOCATION.clone()
}

/// The CLI's composed authentication policy: eager `SYSAND_CRED_*`
/// credentials first, then lazily read stored credentials from the OS
/// keyring.
pub type CliAuthPolicy = StandardLazyHTTPAuthentication<credential_store::CliBlobBackend>;

pub mod cli;
pub mod commands;
pub(crate) mod cred_env;
pub mod credential_store;
pub mod env_vars;
pub mod logger;
pub mod output;
pub mod style;

mod error;
pub use error::CliError;

mod globs;
pub use globs::expand_globs;

/// Whether this invocation owns the process it runs in.
///
/// Two process-wide things care:
///
/// - The global logger. `log` allows one logger per process, and the CLI
///   installs its own; what differs is whether finding the slot already taken
///   is worth telling the user about.
/// - The crash hook, which in release builds asks for a Sysand bug report on
///   any panic. [`lib_main_with`] installs it only when the CLI owns the
///   process; [`run_parsed`] never does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProcessOwnership {
    /// The CLI owns the process, as in the `sysand` binary. Nothing else runs
    /// there, so a logger it could not install is a surprise, and the loss of
    /// its formatting is reported on standard error. Any panic is Sysand's,
    /// so [`lib_main_with`] installs the crash hook.
    #[default]
    Owned,
    /// A host owns the process and may have installed a logger before the
    /// first command ran — for the Python bindings, the `pyo3-log` bridge
    /// every API entry point installs. That is the embedder's arrangement
    /// rather than an accident, so it is not reported; the level
    /// `--verbose`/`--quiet` select still applies to the host's logger, and
    /// only the formatting and the `RUST_LOG` filters are lost. Sysand panic
    /// hook is not installed.
    Embedded,
}

/// Run the CLI and return the exit code the process should report.
///
/// A number rather than a [`std::process::ExitCode`], which can be returned
/// from `main` but not inspected: the Python binding has to pass the code on
/// to a caller of its own. `main.rs` is what wraps it.
///
/// For the process's own binary. An embedder wants [`lib_main_with`].
pub fn lib_main<I, T>(args: I) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    lib_main_with(args, ProcessOwnership::Owned)
}

/// [`lib_main`], told whether the CLI owns the process. See
/// [`ProcessOwnership`].
pub fn lib_main_with<I, T>(args: I, ownership: ProcessOwnership) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    // Before parsing, so the crash hook also covers a panic inside clap.
    claim_process(ownership);

    match Args::try_parse_from(args) {
        Ok(args) => run_parsed(args.global_opts, args.command, ownership),
        Err(err) => {
            err.print().expect("failed to write Clap error");
            // `exit_code()` is non-negative and within u8
            #[expect(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let code = err.exit_code() as u8;
            code
        }
    }
}

/// Run a command that is already parsed, and return the exit code the process
/// should report.
///
/// WARNING: `global_opts` and `command` must obey the invariants from `cli.rs`,
/// which are only given as clap macro attributes.
///
/// For an embedder with a command line of its own, which parses into
/// [`cli::GlobalOptions`] and [`cli::Command`] itself rather than handing
/// Sysand an argument list. Everything after parsing is as in
/// [`lib_main_with`]: the error, if any, is printed to standard error and the
/// code is 1; otherwise it is 0.
///
/// Unlike [`lib_main_with`], it installs no crash hook, whatever `ownership`
/// says: a caller that parses its own command line runs Rust code of its own,
/// and its panics are its own to report.
pub fn run_parsed(
    global_opts: cli::GlobalOptions,
    command: cli::Command,
    ownership: ProcessOwnership,
) -> u8 {
    // One token per run, cancelled by Ctrl-C when this process is ours
    // to receive it. An embedder that runs the CLI in its process gets
    // no signal hook (the hook is never unregistered) and cancels
    // nothing this way.
    let cancellation = CancellationToken::new();
    if matches!(ownership, ProcessOwnership::Owned) {
        install_ctrl_c(cancellation.clone());
    }
    // The run's output format, per run.
    output::begin(global_opts.format);
    let err = match run_cli_with(global_opts, command, ownership, cancellation.clone()) {
        Ok(report) => {
            output::finish_ok(&report);
            return 0;
        }
        Err(err) => err,
    };
    let style = style::ERROR;
    if cancellation.is_cancelled() {
        // Whatever error the cancelled step surfaced, the run was
        // cancelled; 130 is the conventional exit code.
        output::finish_err("cancelled", "cancelled");
        eprintln!("{style}error{style:#}: cancelled");
        return 130;
    }
    output::finish_err(&failure_kind(&err), &format!("{err:#}"));
    eprintln!("{style}error{style:#}: {err}");
    let mut causes = err.chain();
    // The first cause is the error itself which is printed already
    _ = causes.next();
    for cause in causes {
        eprintln!("{style}  caused by:{style:#} {cause}");
    }
    let note_style = style::GOOD;
    if log::max_level() < log::Level::Debug {
        eprintln!("\n{note_style}note{note_style:#}: pass `--verbose` to output additional logs");
    }
    1
}

/// Set up what the process is owed by a CLI that owns it; nothing for one
/// that does not. Safe to call more than once.
fn claim_process(ownership: ProcessOwnership) {
    // The hook is process-wide and blames any panic on Sysand, which is only
    // fair when no other Rust code runs in the process. Not in debug builds,
    // where it clutters panic output.
    if cfg!(not(debug_assertions)) && ownership == ProcessOwnership::Owned {
        use std::sync::Once;

        // Each installation wraps the hook in place, so a second one would
        // print the banner twice.
        static INSTALLED: Once = Once::new();
        INSTALLED.call_once(install_panic_hook);
    }
}

/// Render, as a string, the long help clap would print for the command
/// `args` names, invoked as `prog`.
///
/// `prog` is the program name to spell in usage lines, passed separately
/// rather than as `args[0]`, because callers hold the two apart: an external
/// command is handed the arguments after its own name.
///
/// `args` may name a subcommand path (`["env", "install"]`), and the help
/// returned is that subcommand's. When the arguments are not a valid command
/// line the result is clap's own error text, complete with its usage hint,
/// rather than an empty string — the same thing a user would have seen.
///
/// The text is unstyled. The caller prints it through a writer we know
/// nothing about, so emitting ANSI escapes here would put them into pipes
/// and files. The binary's own `--help` is unaffected.
pub fn render_long_help<I, T>(prog: &str, args: I) -> String
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    /// `Ok` is the rendered help (clap reports it as an error), `Err` means
    /// the command line parsed and produced no help at all.
    fn rendered(argv: Vec<OsString>) -> Result<String, clap::ArgMatches> {
        match Args::command().try_get_matches_from(argv) {
            Err(err) => Ok(err.render().to_string()),
            Ok(matches) => Err(matches),
        }
    }

    // `--help` is a global argument with `ArgAction::HelpLong`, so appending
    // it reaches whatever subcommand the arguments name.
    let argv = iter::once(OsString::from(prog))
        .chain(args.into_iter().map(Into::into))
        .chain(iter::once(OsString::from("--help")))
        .collect();

    match rendered(argv) {
        Ok(help) => help,
        // The appended `--help` was consumed as a value rather than acted
        // on — anything after `--`, or after a `trailing_var_arg`
        // positional, is positional too. Ask again through clap's `help`
        // subcommand, which takes a path of subcommand names and no flags
        // to swallow.
        Err(matches) => {
            let mut argv = vec![OsString::from(prog), OsString::from("help")];
            let mut current = &matches;
            while let Some((name, sub)) = current.subcommand() {
                argv.push(OsString::from(name));
                current = sub;
            }
            rendered(argv).unwrap_or_else(|_| {
                // Not reachable through any command line we know of; a root
                // help labelled with the caller's program name beats a panic
                // in a library.
                Args::command()
                    .bin_name(prog)
                    .render_long_help()
                    .to_string()
            })
        }
    }
}

fn install_panic_hook() {
    use std::panic;
    // TODO: use `panic::update_hook()` once it's stable
    //       also set backtrace style once it's stable, but take
    //       into account the current level
    let default_hook = panic::take_hook();
    // panic::set_backtrace_style(panic::BacktraceStyle::Short);
    panic::set_hook(Box::new(move |panic_info| {
        std::eprintln!(
            "\n\n\
            Sysand crashed. This is likely a bug. We would appreciate a bug report at either\n\
            Sysand issue tracker: https://github.com/sensmetry/sysand/issues\n\
            or Sensmetry forum: https://forum.sensmetry.com/c/sysand/24\n\
            or via email: sysand@sensmetry.com\n\
            \n\
            Below are details of the crash. It would be helpful to include them in the bug report."
        );
        default_hook(panic_info);
    }));
}

/// The eager part of the CLI's credential resolution: every validated
/// `SYSAND_CRED_*` group composed into one policy. Malformed groups are an
/// error naming the variable to fix.
pub fn env_auth_policy() -> Result<StandardHTTPAuthentication> {
    // Validation guarantees every group has a pattern and at least one
    // complete scheme, so iterating the patterns covers every group.
    let groups = cred_env::validated_env_groups()?;
    let mut auths_builder = StandardHTTPAuthenticationBuilder::new();
    for (k, pattern) in &groups.patterns {
        if let (Some(username), Some(password)) =
            (groups.basic_users.get(k), groups.basic_passwords.get(k))
        {
            log::debug!("auth: env vars specify HTTP basic for URL glob `{pattern}`");
            auths_builder.add_basic_auth(pattern, username, password);
        }
        if let Some(token) = groups.bearer_tokens.get(k) {
            log::debug!("auth: env vars specify bearer token for URL glob `{pattern}`");
            // The label lets a publish auth failure name the
            // `SYSAND_CRED_<LABEL>` variable to fix.
            auths_builder.add_bearer_auth(pattern, token, k);
        }
    }
    Ok(auths_builder.build()?)
}

/// The CLI's own credential resolution: [`env_auth_policy`] composed with
/// the lazily read OS credential store that `sysand auth login` writes.
/// Used by `run_cli` and by the language bindings, so a binding
/// authenticates exactly as the CLI does.
///
/// Opening the store only resolves a lock file path; no keychain access
/// happens until a request actually needs a stored credential. An
/// unavailable store is a warning, not an error. With
/// `use_credential_store == false` the store is never opened and only
/// `SYSAND_CRED_*` credentials are used — a policy that can never prompt.
pub fn standard_auth_policy(use_credential_store: bool) -> Result<CliAuthPolicy> {
    let env_auth_policy = env_auth_policy()?;
    if !use_credential_store {
        return Ok(CliAuthPolicy::without_store(env_auth_policy));
    }
    Ok(match credential_store::open_cli_credential_store() {
        Ok(store) => CliAuthPolicy::new(env_auth_policy, store),
        Err(err) => {
            log::warn!(
                "credential store unavailable: {err};\n\
                 continuing with `SYSAND_CRED_*` credentials only"
            );
            CliAuthPolicy::without_store(env_auth_policy)
        }
    })
}

/// The manifest a project's writing commands touch.
fn manifest_path(project: &LocalSrcProject) -> Utf8PathBuf {
    match sysand_core::project::ProjectRead::format(project) {
        Ok(sysand_core::project::ProjectFormat::Toml) => project
            .root_path()
            .join(sysand_core::manifest::MODEL_TOML_FILE),
        _ => project.info_path(),
    }
}

/// The result document of a mutating command: its name and the paths it
/// writes, by kind. A command that fills a document of its own (`info`,
/// `sources`, `env list`, `print-root`, `versions`, `lock`, `sync`)
/// overrides this with it.
fn run_report(
    command: &cli::Command,
    lock_root: &Utf8Path,
    project_root: Option<&Utf8Path>,
    manifest: Option<Utf8PathBuf>,
    cwd: &Utf8Path,
) -> RunReport {
    let lockfile = lock_root.join(DEFAULT_LOCKFILE_NAME);
    let env_dir = lock_root.join(DEFAULT_ENV_NAME);
    let manifest_or_root = || {
        manifest
            .clone()
            .or_else(|| project_root.map(Utf8Path::to_path_buf))
    };
    match command {
        cli::Command::Init {
            path,
            name: _,
            publisher: _,
            version: _,
            license: _,
            metamodel: _,
            metamodel_release: _,
        } => RunReport::new("init").changed(
            path.as_deref()
                .map_or_else(|| cwd.to_path_buf(), |p| cwd.join(p)),
        ),
        cli::Command::Add {
            selector: _,
            locator: _,
            version_constraint: _,
            sync: _,
            resolution_opts: _,
            source_opts: _,
        } => RunReport::new("add")
            .changed_all(manifest_or_root())
            .changed(&lockfile)
            .changed(&env_dir),
        cli::Command::Remove {
            selector: _,
            locator: _,
            sync: _,
            resolution_opts: _,
        } => RunReport::new("remove")
            .changed_all(manifest_or_root())
            .changed(&lockfile)
            .changed(&env_dir),
        cli::Command::Include {
            selector: _,
            paths: _,
            compute_checksum: _,
            no_index_symbols: _,
        } => RunReport::new("include").changed_all(manifest_or_root()),
        cli::Command::Exclude {
            selector: _,
            paths: _,
        } => RunReport::new("exclude").changed_all(manifest_or_root()),
        // `migrate`, `clone` and `publish` decide what they write inside
        // themselves (a deleted json pair, an installed dependency set,
        // the archives found); the report names the command only.
        cli::Command::Migrate {
            path: _,
            publisher: _,
            metamodel: _,
            allow_dirty: _,
            allow_no_vcs: _,
            resolution_opts: _,
        } => RunReport::new("migrate"),
        // `info` reads (a `--get` fills the `values` document); `edit`
        // changes the manifest.
        cli::Command::Info {
            selector: _,
            locator: _,
            get: _,
            resolution_opts: _,
        } => RunReport::new("info"),
        cli::Command::Edit {
            dir: _,
            selector: _,
            edits: _,
        } => RunReport::new("edit").changed_all(manifest_or_root()),
        cli::Command::Build {
            selector: _,
            path: _,
            compression: _,
            allow_path_usage: _,
            keep_index: _,
        } => RunReport::new("build")
            .changed(lock_root.join(sysand_core::commands::build::DEFAULT_OUTPUT_NAME)),
        cli::Command::Publish {
            selector: _,
            path: _,
            index: _,
            trusted_publishing: _,
        } => RunReport::new("publish"),
        cli::Command::Lock { resolution_opts: _ } => RunReport::new("lock").changed(&lockfile),
        cli::Command::Sync {
            resolution_opts: _,
            no_prune: _,
            verify: _,
            repair: _,
        } => RunReport::new("sync").changed(&env_dir),
        cli::Command::Env { command } => match command {
            None => RunReport::new("env").changed(&env_dir),
            Some(cli::EnvCommand::List) => RunReport::new("env list"),
            Some(cli::EnvCommand::Sources {
                locator: _,
                version_constraint: _,
                sources_opts: _,
            }) => RunReport::new("env sources"),
        },
        cli::Command::Index { command } => {
            let (name, root) = match command {
                cli::IndexCommand::Init { index_root } => ("index init", index_root),
                cli::IndexCommand::Add {
                    index_root,
                    iri: _,
                    kpar_path: _,
                } => ("index add", index_root),
                cli::IndexCommand::Yank {
                    index_root,
                    iri: _,
                    version: _,
                } => ("index yank", index_root),
                cli::IndexCommand::Remove {
                    index_root,
                    iri: _,
                    target: _,
                } => ("index remove", index_root),
            };
            RunReport::new(name).changed(root.clone().unwrap_or_else(|| cwd.to_path_buf()))
        }
        cli::Command::Clone {
            locator: _,
            target: _,
            version_constraint: _,
            no_deps: _,
            resolution_opts: _,
        } => RunReport::new("clone"),
        cli::Command::Sources {
            selector: _,
            sources_opts: _,
        } => RunReport::new("sources"),
        cli::Command::PrintRoot { selector: _ } => RunReport::new("print-root"),
        cli::Command::Versions {
            iri: _,
            resolution_opts: _,
        } => RunReport::new("versions"),
        cli::Command::Auth { command: _ } => RunReport::new("auth"),
        cli::Command::Experimental {} => RunReport::new("experimental"),
    }
}

/// The failure document's `kind`: what a reader such as a retry loop
/// needs to tell apart. Anything else is `error`.
fn failure_kind(err: &anyhow::Error) -> String {
    if err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<sysand_core::workspace_lock::LockError>(),
            Some(sysand_core::workspace_lock::LockError::Held { root: _ })
        )
    }) {
        return "lock-held".to_owned();
    }
    if err
        .chain()
        .any(|cause| cause.downcast_ref::<VerifyFailed>().is_some())
    {
        return "verify".to_owned();
    }
    "error".to_owned()
}

/// `sync --verify` found problems: a typed failure so the JSON kind is
/// `verify` and the text is the one message.
#[derive(Debug, thiserror::Error)]
#[error("`.sysand` does not match `sysand-lock.toml`{after}: {count} problem(s){remain}",
    after = .repaired.map_or_else(String::new, |n| format!(" after repairing {n} project(s)")),
    remain = if .repaired.is_some() { " remain" } else { "" })]
pub struct VerifyFailed {
    pub count: usize,
    /// How many projects `--repair` reinstalled; `None` for `--verify`.
    pub repaired: Option<usize>,
}

/// Cancel `token` on the first Ctrl-C and exit with 130 on the second.
///
/// On its own thread with its own small runtime: the command's runtime is
/// current-thread and polls only inside `block_on`, so a handler spawned
/// there would sleep through every synchronous step (an extraction, a
/// clone) and could never offer the second, immediate exit.
#[expect(
    clippy::exit,
    reason = "the second Ctrl-C is the user's demand to stop now, whatever step is in flight"
)]
fn install_ctrl_c(token: CancellationToken) {
    let spawned = std::thread::Builder::new()
        .name("sysand-ctrl-c".to_owned())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                if tokio::signal::ctrl_c().await.is_err() {
                    return;
                }
                eprintln!(
                    "cancelling; the step in flight finishes first (press again to exit now)"
                );
                token.cancel();
                if tokio::signal::ctrl_c().await.is_ok() {
                    std::process::exit(130);
                }
            });
        });
    if let Err(err) = spawned {
        log::debug!("no Ctrl-C handler: {err}");
    }
}

fn run_cli_with(
    global_opts: cli::GlobalOptions,
    command: cli::Command,
    ownership: ProcessOwnership,
    cancellation: CancellationToken,
) -> Result<RunReport> {
    sysand_core::style::set_style_config(crate::style::CONFIG);

    let cwd = wrapfs::current_dir()?;
    let log_level = get_log_level(global_opts.verbose, global_opts.quiet);
    if logger::init(log_level).is_err() {
        // Someone else owns the one global logger. The level still reaches it
        // — `set_max_level` is not the logger's to keep — so `--verbose` and
        // `--quiet` work either way, and what is actually lost is this
        // crate's formatting and the `RUST_LOG` filters `logger::init`
        // parses. Set the level first, so the note below can pass it.
        log::set_max_level(log_level);
        match ownership {
            ProcessOwnership::Owned => {
                let warn = style::WARN;
                eprintln!(
                    "{warn}warning{warn:#}: failed to set up logger because it has already been set up;\n\
                    {:>8} log messages may not be formatted properly",
                    ' '
                );
            }
            // Every invocation in the host's process would say this, about a
            // situation the host arranged on purpose. Still logged, but in debug,
            // because it does explain why the output looks unlike the binary's.
            ProcessOwnership::Embedded => log::debug!(
                "the host installed the global logger first; sysand's own formatting and \
                 `RUST_LOG` filters do not apply"
            ),
        }
    }
    log::debug!("sysand v{}", env!("CARGO_PKG_VERSION"));

    let current_project = discover_project(&cwd)?;
    let project_root = current_project.as_ref().map(|p| p.root_path().to_owned());

    // One upward walk for both questions below: configuration is read
    // at the workspace root, and a command with a scope has to know
    // which workspace it is in before it runs.
    //
    // Ahead of the environment, which is rooted at the workspace: a
    // single `.sysand` per workspace cannot be chosen before the
    // workspace is known. `config_root` stays below, since it needs
    // `project_root`.
    let discovered = discover_table(&cwd);
    let workspace_root = match &discovered {
        Ok(Some((root, _))) => Some(root.clone()),
        Ok(None) => None,
        // Not fatal *here*: see `config_root`.
        Err(error) => {
            log::debug!("not reading configuration from a workspace root: {error}");
            None
        }
    };

    // Nesting and unlisted projects, for the commands that have a scope.
    // The workspace root and where under it the command started: both
    // halves are needed, since `StartRegion` carries no root.
    //
    // The table is kept, not just consulted: a member's manifest cannot
    // be read without it, and `current_project` below is built with it.
    let mut workspace_table: Option<(Utf8PathBuf, Arc<WorkspaceTable>)> = None;
    let scope: Option<(Utf8PathBuf, StartRegion)> = if consults_the_workspace(&command) {
        match discovered {
            Ok(Some((root, table))) => {
                let start = start_region(&root, &table, &cwd)?;
                workspace_table = Some((root.clone(), Arc::new(table)));
                Some((root, start))
            }
            Ok(None) => None,
            Err(error) => return Err(error.into()),
        }
    } else {
        None
    };

    // `model-toml.md` "Inheritance": a marker resolves against the
    // workspace table, and only for a member. Without the table, a
    // member that inherits cannot be read as the current project.
    //
    // `member_index` and not `scope`: `StartRegion::Member` prefix-
    // matches, so it also holds for a project nested *inside* a member,
    // which is not one. `None` as the nominal path is what makes
    // `sources` relativize against the workspace root -- the directory the shared lockfile's
    // `src_path`s are relative to.
    // workspaces.md "Command scopes": a selector replaces which member
    // the command acts on, so it is applied before `current_project` is
    // settled rather than inside each arm. `Current` for every command
    // that takes no selector, and for every invocation that gives none.
    let selector = match &command {
        Command::Build {
            selector,
            path: _,
            compression: _,
            allow_path_usage: _,
            keep_index: _,
        }
        | Command::Publish {
            selector,
            path: _,
            index: _,
            trusted_publishing: _,
        }
        | Command::Info {
            selector,
            locator: _,
            get: _,
            resolution_opts: _,
        }
        | Command::Sources {
            selector,
            sources_opts: _,
        } => selector.selector(),
        Command::Add {
            selector,
            locator: _,
            version_constraint: _,
            sync: _,
            resolution_opts: _,
            source_opts: _,
        }
        | Command::Remove {
            selector,
            locator: _,
            sync: _,
            resolution_opts: _,
        }
        | Command::Include {
            selector,
            paths: _,
            compute_checksum: _,
            no_index_symbols: _,
        }
        | Command::Exclude { selector, paths: _ }
        | Command::PrintRoot { selector }
        | Command::Edit {
            selector,
            dir: _,
            edits: _,
        } => selector.selector(),
        _ => Selector::Current,
    };
    let selected = selected_member(&cwd, workspace_root.as_deref(), selector)?;
    let current_project = match (&selected, &workspace_table) {
        (Some(dir), Some((_, table))) => Some(LocalSrcProject::new_member(
            dir.clone(),
            None,
            table.clone(),
        )),
        // "`--workspace` forces all members": the commands that have an
        // all-members path take it when there is no current project, so
        // forcing it is dropping the one the working directory implies.
        _ if selector == Selector::Workspace && workspace_root.is_some() => None,
        _ => current_project,
    };
    let project_root = selected.clone().or(project_root);

    // workspaces.md "Command scopes", second row: these commands "need
    // a project, so `--member` is required at the root". Checked once
    // here rather than in each command, because the condition is one
    // condition: in a workspace, with no member to act on.
    //
    // `edit --dir` names its project directly, so only a bare `edit`
    // belongs to the row.
    let needs_a_member = matches!(
        &command,
        Command::Add {
            selector: _,
            locator: _,
            version_constraint: _,
            sync: _,
            resolution_opts: _,
            source_opts: _
        } | Command::Remove {
            selector: _,
            locator: _,
            sync: _,
            resolution_opts: _
        } | Command::Include {
            selector: _,
            paths: _,
            compute_checksum: _,
            no_index_symbols: _
        } | Command::Exclude {
            selector: _,
            paths: _
        } | Command::Edit {
            dir: None,
            selector: _,
            edits: _
        }
    );
    if needs_a_member && current_project.is_none() && workspace_root.is_some() {
        return Err(DiscoverError::NeedsProject.into());
    }

    let current_project = match (current_project, &workspace_table) {
        (Some(project), Some((root, table)))
            if member_index(root, table, project.root_path())?.is_some() =>
        {
            Some(LocalSrcProject::new_member(
                project.root_path().to_owned(),
                None,
                table.clone(),
            ))
        }
        (project, _) => project,
    };

    // `workspaces.md` "Syncing": "Always a single `.sysand` directory in
    // workspace root." From `scope`, not from `workspace_root`: the
    // latter is `Some` for any discovered table, an `exclude`d
    // standalone project included, and such a project keeps its own
    // state.
    let model_workspace_root = scope.as_ref().and_then(|(root, start)| {
        matches!(start, StartRegion::Member(_) | StartRegion::Workspace).then(|| root.clone())
    });

    // A mutating command holds the workspace's advisory lock for the
    // whole invocation, taken here -- once, before the environment is
    // read -- and handed to every environment it writes. `lock_root` is where `sysand-lock.toml` and `.sysand` live.
    let lock_root = model_workspace_root
        .clone()
        .or_else(|| project_root.clone())
        .unwrap_or_else(|| cwd.clone());
    let workspace_lock = match lock_mode(&command, &lock_root) {
        Some(mode) => Some(Arc::new(WorkspaceLock::acquire(
            &lock_root,
            mode,
            global_opts.lock_wait.map(Duration::from_secs),
        )?)),
        None => None,
    };

    let env = match (&model_workspace_root, &current_project) {
        (Some(root), _) => get_env(root)?,
        (None, Some(pr)) => get_env(pr.root_path())?,
        // TODO: does it make sense to support env unassociated with a project
        // when index and env are different?
        (None, None) => get_env(&cwd)?,
    };
    // A shared lock is not handed on: the environment writes under an
    // exclusive one only
    let env = match (env, workspace_lock.clone().and_then(ExclusiveLock::new)) {
        (Some(mut env), Some(lock)) => {
            env.hold(lock)?;
            Some(env)
        }
        (env, _) => env,
    };
    let config_root = config_root(&cwd, project_root.as_deref(), workspace_root.as_deref());
    let ctx = ProjectContext {
        env,
        workspace_lock,
        cancellation,
        model_workspace_root: model_workspace_root.clone(),
        current_project,
        current_directory: cwd,
        config_root,
    };

    let auto_config = if global_opts.no_config {
        Config::default()
    } else {
        // workspaces.md "Source overrides": a `sysand.toml` under the
        // workspace but not at its root is ignored with a warning.
        // `--no-config` silences it too: warning
        // about a file the run was told to ignore is noise.
        if reads_configuration(&command) {
            for stray in stray_configs(&ctx.current_directory, &ctx.config_root)? {
                log::warn!(
                    "{}",
                    ReadWarning::IgnoredSourceOverrides {
                        path: stray.to_string()
                    }
                );
            }
        }
        load_configs(&ctx.config_root)?
    };

    let mut config = match &global_opts.config_file {
        // `add` creates the file to write the source to.
        Some(config_file) if matches!(&command, Command::Add { selector: _, source_opts, locator: _, version_constraint: _, sync: _, resolution_opts: _ } if source_opts.is_given()) => {
            get_config(config_file)?
        }
        Some(config_file) => read_config(config_file)?,
        None => Config::default(),
    };

    config.merge(auto_config);

    let client = create_reqwest_client()?;

    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap(),
    );

    let _runtime_keep_alive = runtime.clone();

    // The `auth` commands manage credentials rather than use them, so
    // they dispatch before the eager env-policy build below. `status`
    // applies the same strict `SYSAND_CRED_*` validation on its own;
    // `login`, `logout`, and `whoami` must keep working even when those
    // variables are malformed. `login` still gets the shared client and
    // runtime: core
    // handles its discovery fetch itself (an unauthenticated baseline
    // with a forced-bearer retry carrying the just-entered secret), so
    // no ambient auth policy is needed.
    let command = match command {
        Command::Auth { command } => {
            // `login` prompts, and `status` and `whoami` have no
            // document (their text form is the only output they have);
            // the others report a result.
            if output::is_json() && !matches!(command, AuthCommand::Logout { index_url: _ }) {
                let name = match command {
                    AuthCommand::Login {
                        index_url: _,
                        token_stdin: _,
                    } => "auth login",
                    AuthCommand::Whoami { index_url: _ } => "auth whoami",
                    _ => "auth status",
                };
                bail!("{}", output::not_supported(name));
            }
            return match command {
                AuthCommand::Status => {
                    command_auth_status(&config).map(|()| RunReport::new("auth status"))
                }
                AuthCommand::Login {
                    index_url,
                    token_stdin,
                } => command_auth_login(index_url, token_stdin, &config, &client, runtime)
                    .map(|()| RunReport::new("auth login")),
                AuthCommand::Whoami { index_url } => {
                    command_auth_whoami(index_url, &config, &client, &runtime)
                        .map(|()| RunReport::new("auth whoami"))
                }
                AuthCommand::Logout { index_url } => {
                    command_auth_logout(index_url, &config).map(|()| RunReport::new("auth logout"))
                }
            };
        }
        command => command,
    };

    let auth_policy = Arc::new(standard_auth_policy(true)?);

    // What a mutating command reports having changed, by kind, computed
    // before the arguments move into the arms.
    let report = run_report(
        &command,
        &lock_root,
        project_root.as_deref(),
        ctx.current_project.as_ref().map(manifest_path),
        &ctx.current_directory,
    );

    let outcome: Result<()> = match command {
        Command::Init {
            path,
            name,
            publisher,
            version,
            license,
            metamodel,
            metamodel_release,
        } => {
            let metamodel = metamodel
                .map(|metamodel| parse_metamodel(&metamodel, metamodel_release))
                .transpose()?;
            command_init(name, publisher, version, license, metamodel, path, ctx)
        }
        Command::Env { command } => match command {
            None => {
                let env_dir = {
                    // workspaces.md "Command scopes": `env` is "per
                    // workspace: the single `.sysand`". The workspace
                    // root first, or a member's `sysand env` would make
                    // a second environment beside the one every other
                    // command shares.
                    let mut p = model_workspace_root
                        .or(project_root)
                        .unwrap_or(ctx.current_directory);
                    p.push(DEFAULT_ENV_NAME);
                    p
                };
                command_env(env_dir)?;

                Ok(())
            }
            // Some(EnvCommand::Install {
            //     iri,
            //     version,
            //     path,
            //     install_opts,
            //     resolution_opts,
            // }) => {
            //     if let Some(path) = path {
            //         command_env_install_path(
            //             iri,
            //             version,
            //             path,
            //             install_opts,
            //             resolution_opts,
            //             &config,
            //             project_root,
            //             client,
            //             runtime,
            //             auth_policy,
            //             ctx,
            //         )
            //     } else {
            //         command_env_install(
            //             iri,
            //             version,
            //             install_opts,
            //             resolution_opts,
            //             &config,
            //             project_root,
            //             client,
            //             runtime,
            //             auth_policy,
            //             ctx,
            //         )
            //     }
            // }
            // Some(EnvCommand::Uninstall { iri, version }) => {
            //     if let Some(local_environment) = ctx.env {
            //         command_env_uninstall(iri, version, local_environment)
            //     } else {
            //         log::warn!("no environment to uninstall from");
            //         Ok(())
            //     }
            // }
            Some(EnvCommand::List) => command_env_list(ctx.env),
            Some(EnvCommand::Sources {
                locator,
                version_constraint,
                sources_opts,
            }) => command_sources_env(
                *locator,
                version_constraint,
                sources_opts.no_own,
                sources_opts.deps,
                ctx.env,
            ),
        },
        Command::Index { command } => {
            let root =
                |index_root: Option<Utf8PathBuf>| index_root.unwrap_or(ctx.current_directory);
            match command {
                IndexCommand::Init { index_root } => command_index_init(root(index_root))?,
                IndexCommand::Add {
                    iri,
                    kpar_path,
                    index_root,
                } => command_index_add(iri, kpar_path, root(index_root))?,
                IndexCommand::Yank {
                    iri,
                    version,
                    index_root,
                } => command_index_yank(iri, version, root(index_root))?,
                IndexCommand::Remove {
                    iri,
                    target,
                    index_root,
                } => {
                    let target = match (target.version, target.project) {
                        (Some(version), false) => RemoveTarget::Version(version),
                        (None, true) => RemoveTarget::Project,
                        _ => unreachable!(),
                    };
                    command_index_remove(iri, target, root(index_root))?
                }
            }
            Ok(())
        }
        Command::Lock { resolution_opts } => {
            // `workspaces.md` "Locking": one lockfile for the whole
            // workspace, at its root, from anywhere under it.
            if let Some(root) = &model_workspace_root {
                let lock = command_lock_workspace(
                    root,
                    resolution_opts,
                    &config,
                    client,
                    runtime,
                    auth_policy,
                    &ctx,
                )?;
                output::lock_document(&lock, &root.join(DEFAULT_LOCKFILE_NAME));
                Ok(())
            } else if let Some(project_root) = project_root {
                let lock = command_lock(
                    ".",
                    resolution_opts,
                    &config,
                    &project_root,
                    client,
                    runtime,
                    auth_policy,
                    &ctx,
                )?;
                output::lock_document(&lock, &project_root.join(DEFAULT_LOCKFILE_NAME));
                Ok(())
            } else {
                bail!(
                    "not inside a project - neither current nor any of the parent directories contain a SysML v2 or KerML project"
                )
            }
        }
        Command::Sync {
            resolution_opts,
            no_prune,
            verify,
            repair,
        } => {
            let provided_iris = if resolution_opts.include_std {
                HashMap::default()
            } else {
                known_std_libs()
            };

            // The lockfile, and the root every `src_path` in it is
            // relative to, are the workspace's when there is one.
            let project_root = model_workspace_root
                .clone()
                .or(project_root)
                .unwrap_or_else(|| ctx.current_directory.clone());
            let lock = match Lockfile::try_read(&project_root)? {
                Some(lockfile) => {
                    lockfile.lock().warn_release_mismatches();
                    lockfile.into_lock()
                }
                // The same branch as `Command::Lock`: locking a workspace
                // one member at a time would write a single-member solve
                // over the shared file.
                None => match &model_workspace_root {
                    Some(root) => command_lock_workspace(
                        root,
                        resolution_opts,
                        &config,
                        client.clone(),
                        runtime.clone(),
                        auth_policy.clone(),
                        &ctx,
                    )?,
                    None => command_lock(
                        ".",
                        resolution_opts,
                        &config,
                        &project_root,
                        client.clone(),
                        runtime.clone(),
                        auth_policy.clone(),
                        &ctx,
                    )?,
                },
            };
            if verify || repair {
                // `--verify` reads; it must not create a `.sysand` that
                // was not there (with the directory gone every locked
                // project is reported as not installed).
                let manifests = ManifestUsages::for_context(&ctx)?;
                let env_dir = project_root.join(DEFAULT_ENV_NAME);
                let mut outcome = SyncOutcome::default();
                let status = command_sync_verify(
                    &lock,
                    &project_root,
                    ctx.env,
                    &env_dir,
                    &manifests,
                    client,
                    &provided_iris,
                    runtime,
                    auth_policy,
                    no_prune,
                    repair,
                    ctx.cancellation.clone(),
                    &mut outcome,
                )?;
                output::document(
                    "verify",
                    match status {
                        VerifyStatus::Clean {
                            verified,
                            repaired: _,
                        } => {
                            serde_json::json!({ "clean": true, "problems": 0, "verified": verified })
                        }
                        VerifyStatus::Problems { count, repaired: _ } => {
                            serde_json::json!({ "clean": false, "problems": count, "verified": outcome.verified.len() })
                        }
                    },
                );
                output::sync_document(&outcome);
                return match status {
                    VerifyStatus::Clean {
                        verified: _,
                        repaired: _,
                    } => Ok(report),
                    VerifyStatus::Problems { count, repaired } => Err(VerifyFailed {
                        count,
                        repaired: repair.then_some(repaired),
                    }
                    .into()),
                };
            }
            let mut local_environment = get_or_create_env(
                ctx.env,
                ctx.model_workspace_root.as_deref(),
                ctx.current_project.as_ref(),
                &ctx.current_directory,
                ctx.workspace_lock.as_ref(),
            )?;
            let mut outcome = SyncOutcome::default();
            command_sync(
                &lock,
                project_root,
                &mut local_environment,
                client,
                &provided_iris,
                runtime,
                auth_policy,
                no_prune,
                ctx.cancellation.clone(),
                &mut outcome,
            )?;
            output::sync_document(&outcome);
            Ok(())
        }
        Command::Auth { command: _ } => {
            unreachable!("`auth` is dispatched before the auth policy is built")
        }
        // workspaces.md "Command scopes": `print-root` prints the
        // workspace root, and `--member` "prints the member's".
        Command::PrintRoot { selector: _ } => match &selected {
            Some(dir) => {
                output::document("root", dir.as_str());
                output::text(|| println!("{dir}"));
                Ok(())
            }
            None => command_print_root(
                &ctx.current_directory,
                scope.as_ref().map(|(root, start)| (root.as_path(), start)),
            ),
        },
        Command::Info {
            selector,
            locator,
            get,
            // no_normalise,
            resolution_opts,
        } => {
            enum Location {
                WorkDir,
                Resolve(ResolutionInfo),
                Path(Utf8PathBuf, LocalProjectKind),
            }

            let cli::ResolutionOptions {
                index,
                default_index,
                no_index,
                include_std,
                // `info` does not solve
            } = resolution_opts;
            let index_urls = if no_index {
                None
            } else {
                Some(config.index_urls(index, vec![default_index_location()], default_index))
            };
            let excluded_usages: HashSet<_> = if include_std {
                HashSet::default()
            } else {
                known_std_libs().into_keys().collect()
            };

            // A relative override path is relative to the file it is
            // written in, which in a workspace is the root's.
            let overrides = get_overrides(
                &config,
                &ctx.config_root,
                &client,
                runtime.clone(),
                auth_policy.clone(),
            )?;

            let location = match locator {
                cli::InfoProjectLocatorArgs {
                    identifier: Some((publisher, name)),
                    dir: None,
                    kpar_path: None,
                    iri: None,
                } => Location::Resolve(ResolutionInfo::new(
                    InterchangeProjectUsage::Index {
                        publisher,
                        name,
                        // Any version, the highest is described
                        version_constraint: semver::VersionReq::STAR,
                    },
                    None,
                )),
                cli::InfoProjectLocatorArgs {
                    identifier: None,
                    dir: Some(dir),
                    kpar_path: None,
                    iri: None,
                } => Location::Path(dir, LocalProjectKind::Dir),
                cli::InfoProjectLocatorArgs {
                    identifier: None,
                    dir: None,
                    kpar_path: Some(kpar_path),
                    iri: None,
                } => Location::Path(kpar_path, LocalProjectKind::Kpar),
                cli::InfoProjectLocatorArgs {
                    identifier: None,
                    dir: None,
                    kpar_path: None,
                    iri: Some(iri),
                } => Location::Resolve(ResolutionInfo::iri(iri)),
                cli::InfoProjectLocatorArgs {
                    identifier: None,
                    dir: None,
                    kpar_path: None,
                    iri: None,
                } => Location::WorkDir,
                _ => unreachable!(),
            };

            // `--dir`/`--kpar-path`/`--iri` name their own target, so a
            // workspace selector alongside one is a contradiction rather
            // than a precedence question.
            if !matches!(location, Location::WorkDir) && selector.selector() != Selector::Current {
                bail!(
                    "`--workspace` and `--member` select a member of the current workspace, \
                     which `--dir`, `--kpar-path` and `--iri` already answer; pass one or the other"
                );
            }

            match (location, get) {
                (Location::WorkDir, get) => {
                    if let Some(current_project) = ctx.current_project {
                        if let Some(field) = get {
                            command_info_field(&current_project, field)
                        } else {
                            // The project is the one `run_cli`
                            // discovered, which carries the workspace
                            // table a member needs; `command_info_path`
                            // would re-open the directory and throw it
                            // away. The root is copied out before the
                            // project moves.
                            let root = current_project.root_path().to_owned();
                            command_info_project(
                                FileResolverProject::LocalSrcProject(current_project),
                                &root,
                                &excluded_usages,
                            )
                        }
                    } else if let Some(root) = ctx.model_workspace_root.clone() {
                        // workspaces.md "Command scopes": `info` takes
                        // all members from the root or a non-member
                        // directory under it. Reading is all it does,
                        // so there is no writing verb to refuse here.
                        crate::commands::info::command_info_workspace(&root, get, &excluded_usages)
                    } else {
                        bail!(
                            "run outside of an active project, did you mean to use `--dir`, `--kpar-path` or `--iri`?"
                        )
                    }
                }
                (Location::Resolve(resolve), None) => crate::commands::info::command_info_resolve(
                    resolve,
                    true, // !no_normalise,
                    client,
                    index_urls,
                    &excluded_usages,
                    overrides,
                    runtime,
                    auth_policy,
                    ctx,
                ),
                (Location::Resolve(resolve), Some(field)) => {
                    crate::commands::info::command_info_field_resolve(
                        resolve,
                        field,
                        client,
                        index_urls,
                        overrides,
                        runtime,
                        auth_policy,
                        ctx,
                    )
                }
                (Location::Path(path, kind), None) => {
                    command_info_path(&path, kind, &excluded_usages)
                }
                (Location::Path(path, kind), Some(field)) => {
                    command_info_field_path(&path, kind, field)
                }
            }
        }
        Command::Edit {
            dir,
            edits,
            selector: _,
        } => {
            let project = match dir {
                Some(dir) => {
                    if !wrapfs::metadata(&dir)?.is_dir() {
                        bail!("`{dir}` is not a directory");
                    }
                    LocalSrcProject::new_access(dir, None)
                }
                None => ctx
                    .current_project
                    .ok_or(CliError::MissingProjectCurrentDir)?,
            };
            command_edit(project, *edits)
        }
        Command::Add {
            selector: _,
            locator,
            version_constraint,
            resolution_opts,
            source_opts,
            sync,
        } => command_add(
            locator,
            version_constraint,
            sync.no_lock,
            sync.no_sync,
            sync.no_prune,
            resolution_opts,
            source_opts,
            config,
            global_opts.config_file,
            global_opts.no_config,
            ctx,
            client,
            runtime,
            auth_policy,
        )
        .map(|_added| ()),
        Command::Remove {
            selector: _,
            locator,
            sync,
            resolution_opts,
        } => command_remove(
            locator,
            ctx,
            config,
            global_opts.config_file,
            global_opts.no_config,
            sync.no_lock,
            sync.no_sync,
            sync.no_prune,
            resolution_opts,
            client,
            runtime,
            auth_policy,
        ),
        Command::Include {
            selector: _,
            paths,
            compute_checksum: add_checksum,
            no_index_symbols,
        } => command_include(paths, add_checksum, !no_index_symbols, ctx),
        Command::Exclude { selector: _, paths } => command_exclude(paths, ctx),
        Command::Build {
            selector: _,
            path,
            compression,
            allow_path_usage,
            keep_index,
        } => {
            // `build` reads the lockfile of its scope when one exists and
            // validates it before building, so the edge checks run on
            // `sync` and `build`. Parsing *is* the validation --
            // `Lock::from_str` calls `validate` -- so reading the file is
            // the whole of it.
            // Without a lockfile `build` proceeds: it resolves no
            // usages, and the checks run at the next `lock`.
            //
            // The root is computed before `ctx.current_project` moves
            // below. A member never holds its own lockfile, so the
            // workspace root has to win.
            let lock_root = model_workspace_root.clone().or_else(|| {
                ctx.current_project
                    .as_ref()
                    .map(|p| p.root_path().to_owned())
            });
            if let Some(root) = lock_root {
                // Parsing is the validation
                if let Some(lockfile) = Lockfile::try_read(&root)? {
                    lockfile.lock().warn_release_mismatches();
                }
            }
            if let Some(current_project) = ctx.current_project {
                // Even if we are in a workspace, the project takes precedence.
                //
                // The workspace root still decides *where* the archive
                // goes: a workspace has one `output/`, so building a
                // member from inside it writes where building the whole
                // workspace would have.
                let path = if let Some(path) = path {
                    path
                } else {
                    let path = sysand_core::build::default_kpar_path(
                        &current_project,
                        model_workspace_root.as_deref(),
                        current_project.root_path(),
                    )?;
                    if let Some(output_dir) = path.parent()
                        && !wrapfs::is_dir(output_dir)?
                    {
                        wrapfs::create_dir(output_dir)?;
                    }
                    path
                };
                command_build_for_project(
                    path,
                    compression.into(),
                    current_project,
                    !keep_index,
                    allow_path_usage,
                )
            } else {
                // If the workspace is also missing, report an error about
                // missing project because that is what the user is more likely
                // to be looking for.
                let root = model_workspace_root.ok_or(CliError::MissingProjectCurrentDir)?;
                let workspace = Workspace::load_with(&root, MemberChecks::All)?;
                let output_dir =
                    path.unwrap_or_else(|| root.join(sysand_core::build::DEFAULT_OUTPUT_NAME));

                // workspaces.md "Stray per-member state": a member has
                // no `output` of its own. Refused before the directory
                // is created, or this command would itself write the
                // stray state that section refuses.
                let absolute = if output_dir.is_absolute() {
                    output_dir.clone()
                } else {
                    ctx.current_directory.join(&output_dir)
                };
                if let Some(member) = workspace
                    .members()
                    .iter()
                    .find(|member| absolute.starts_with(&member.dir))
                {
                    bail!(
                        "`{output_dir}` is inside the member `{}`; a workspace has one build \
                         output directory, at `{root}`",
                        member.path.raw
                    );
                }

                if !wrapfs::is_dir(&output_dir)? {
                    wrapfs::create_dir(&output_dir)?;
                }
                command_build_for_workspace(
                    output_dir,
                    compression.into(),
                    &workspace,
                    !keep_index,
                    allow_path_usage,
                )
            }
        }
        Command::Migrate {
            path,
            publisher,
            metamodel,
            allow_dirty,
            allow_no_vcs,
            resolution_opts,
        } => crate::commands::migrate::command_migrate(
            path,
            publisher,
            metamodel,
            allow_dirty,
            allow_no_vcs,
            resolution_opts,
            &config,
            &ctx,
            client,
            runtime,
            auth_policy,
        ),
        Command::Publish {
            selector: _,
            path,
            index,
            trusted_publishing,
        } => command_publish(
            path,
            index,
            trusted_publishing,
            &ctx,
            auth_policy,
            client,
            runtime,
        ),
        Command::Versions {
            iri,
            resolution_opts,
        } => {
            let cli::ResolutionOptions {
                index,
                default_index,
                no_index,
                include_std: _,
            } = resolution_opts;
            let index_urls = if no_index {
                None
            } else {
                Some(config.index_urls(index, vec![default_index_location()], default_index))
            };
            crate::commands::versions::command_versions(
                iri,
                client,
                index_urls,
                runtime,
                auth_policy,
                ctx,
            )
        }
        Command::Sources {
            selector: _,
            sources_opts,
        } => match (&ctx.current_project, ctx.model_workspace_root.clone()) {
            // workspaces.md "Command scopes": all members from the root
            // or from a non-member directory under it, which is exactly
            // "a workspace, and no member the directory implies".
            (None, Some(root)) => {
                command_sources_workspace(&root, sources_opts.no_own, sources_opts.deps, ctx)
            }
            _ => command_sources_project(sources_opts.no_own, sources_opts.deps, ctx),
        },
        Command::Clone {
            locator,
            version_constraint,
            target,
            resolution_opts,
            no_deps,
        } => command_clone(
            locator,
            version_constraint,
            target,
            ctx,
            no_deps,
            resolution_opts,
            &config,
            client,
            runtime,
            auth_policy,
        ),
        Command::Experimental {} => {
            bail!("No experimental commands are available");
        }
    };
    outcome?;
    Ok(report)
}

pub fn iri_or_path_to_iri(
    iri: Option<Iri<String>>,
    path: Option<Utf8PathBuf>,
) -> Result<Iri<String>, anyhow::Error> {
    Ok(if let Some(iri) = iri {
        iri
    } else {
        let Some(path) = path else { unreachable!() };
        let abs_path = wrapfs::canonicalize(&path)?;
        let url: String = Url::from_file_path(abs_path)
            .map_err(|()| anyhow!("unsupported path type of `{path}`"))?
            .into();
        Iri::parse(url).expect("BUG: file URL from path is invalid IRI")
    })
}

/// Read `root/.sysand/` metadata
pub fn get_env(
    root: impl AsRef<Utf8Path>,
) -> Result<Option<LocalDirectoryEnvironment>, EnvMetadataError> {
    LocalDirectoryEnvironment::try_read(root.as_ref().join(DEFAULT_ENV_NAME))
}

/// Unpack `env`, or create an empty environment otherwise
pub fn get_or_create_env(
    env: Option<LocalDirectoryEnvironment>,
    model_workspace_root: Option<&Utf8Path>,
    project: Option<&LocalSrcProject>,
    cwd: impl AsRef<Utf8Path>,
    lock: Option<&Arc<WorkspaceLock>>,
) -> Result<LocalDirectoryEnvironment> {
    if let Some(env) = env {
        return Ok(env);
    }
    // workspaces.md "Syncing": one `.sysand` per workspace, at its root.
    let base_path = match (model_workspace_root, project) {
        (Some(root), _) => root,
        (None, Some(pr)) => pr.root_path(),
        (None, None) => cwd.as_ref(),
    };
    let mut env = command_env(base_path.join(DEFAULT_ENV_NAME))?;
    if let Some(lock) = lock.cloned().and_then(ExclusiveLock::new) {
        env.hold(lock)?;
    }
    Ok(env)
}

/// Which advisory lock a command takes: exclusive for the commands that
/// write `sysand-lock.toml` or `.sysand`, shared for `sync --verify`
/// (which must not read a torn state), none for commands that write only
/// a manifest or read. `clone` populates a directory that must be empty
/// or absent and takes none.
fn lock_mode(command: &cli::Command, lock_root: &Utf8Path) -> Option<LockMode> {
    match command {
        // `sync` with no lockfile locks first (writes), so the shared
        // mode is only for a verification that has one to read.
        cli::Command::Sync {
            verify: true,
            repair: false,
            resolution_opts: _,
            no_prune: _,
        } if lock_root.join(DEFAULT_LOCKFILE_NAME).is_file() => Some(LockMode::Shared),
        cli::Command::Lock { resolution_opts: _ }
        | cli::Command::Sync {
            resolution_opts: _,
            no_prune: _,
            verify: _,
            repair: _,
        }
        | cli::Command::Add {
            selector: _,
            locator: _,
            version_constraint: _,
            sync: _,
            resolution_opts: _,
            source_opts: _,
        }
        | cli::Command::Remove {
            selector: _,
            locator: _,
            sync: _,
            resolution_opts: _,
        } => Some(LockMode::Exclusive),
        // A bare `env` creates the environment; the subcommands read it
        cli::Command::Env { command } => match command {
            None => Some(LockMode::Exclusive),
            Some(_) => None,
        },
        _ => None,
    }
}

fn get_log_level(verbose: bool, quiet: bool) -> log::LevelFilter {
    match (verbose, quiet) {
        (true, true) => unreachable!(),
        (true, false) => log::LevelFilter::Debug,
        (false, true) => log::LevelFilter::Error,
        (false, false) => log::LevelFilter::Info,
    }
}

pub type Overrides<Policy> = Vec<(Identifier, Vec<OverrideProject<Policy>>)>;

/// workspaces.md "Source overrides": "A `sysand.toml` anywhere else
/// under the workspace is ignored as a whole -- its `[[index]]`
/// configuration included -- with a warning on any operation touching
/// the overrides or the indexes."
///
/// Those operations are `lock`, `sync`, `add`, `remove`, `clone`, `env`,
/// `info` in a form that resolves, and `publish`. `publish` is passed no `Config` at all and
/// warns because the specification says so rather than because of the
/// code.
///
/// `info` resolves under `--iri` and under a `<publisher>/<name>`
/// identifier, which is looked up in the indexes.
fn reads_configuration(command: &cli::Command) -> bool {
    match command {
        cli::Command::Lock { resolution_opts: _ }
        | cli::Command::Sync { resolution_opts: _, no_prune: _, verify: _, repair: _ }
        | cli::Command::Versions { iri: _, resolution_opts: _ }
        | cli::Command::Add { selector: _, locator: _, version_constraint: _, sync: _, resolution_opts: _, source_opts: _ }
        | cli::Command::Remove { selector: _, locator: _, sync: _, resolution_opts: _ }
        | cli::Command::Clone { locator: _, target: _, version_constraint: _, no_deps: _, resolution_opts: _ }
        | cli::Command::Env { command: _ }
        | cli::Command::Publish { selector: _, path: _, index: _, trusted_publishing: _ }
        // `migrate` builds a resolver: it resolves `http(s)` and
        // `pkg:sysand` usages through an index. A one-time conversion is
        // also a bad place to be quiet about configuration that is not
        // read.
        | cli::Command::Migrate { path: _, publisher: _, metamodel: _, allow_dirty: _, allow_no_vcs: _, resolution_opts: _ } => true,
        cli::Command::Info { locator, selector: _, get: _, resolution_opts: _ } => locator.iri.is_some() || locator.identifier.is_some(),
        _ => false,
    }
}

/// Whether a command consults the `[workspace]` table before it runs.
///
/// `workspaces.md` "Command scopes" gives every command a target --
/// the whole workspace, or the member the command was run in -- except
/// the last row:
///
/// > `clone`, `index`, `auth` | scope-free: `clone` creates a project
/// > outside any scope, `index`/`auth` are configuration
///
/// So a workspace that is nested, or that the command was started
/// outside of, stops the others and not those three. `init` joins them
/// for a different reason: it reads the root table itself, and creating
/// a project must not be blocked by an unrelated member's mistake
/// (`init_in_a_workspace_with_a_broken_member_still_works`).
///
/// This is *consulting*, not loading. `Workspace::load` also enforces
/// the member requirements and "Stray per-member state", so a command
/// that writes a lockfile, a `.sysand` directory or `output` into the
/// member it acts on would, through the loader, refuse the workspace its
/// own previous run created. See `StartRegion`.
///
/// The same paragraph says a project under the root that is neither a
/// member nor excluded is an error "whatever the command". That cannot
/// be read literally, since the paragraph also calls `auth` scope-free:
/// it contrasts the scoped commands with each other, rather than
/// extending the check to commands that have no scope.
fn consults_the_workspace(command: &cli::Command) -> bool {
    match command {
        cli::Command::Build { selector: _, path: _, compression: _, allow_path_usage: _, keep_index: _ }
        | cli::Command::Sources { selector: _, sources_opts: _ }
        | cli::Command::Info { selector: _, locator: _, get: _, resolution_opts: _ }
        | cli::Command::Edit { dir: _, selector: _, edits: _ }
        | cli::Command::Publish { selector: _, path: _, index: _, trusted_publishing: _ }
        | cli::Command::Add { selector: _, locator: _, version_constraint: _, sync: _, resolution_opts: _, source_opts: _ }
        | cli::Command::Remove { selector: _, locator: _, sync: _, resolution_opts: _ }
        | cli::Command::Include { selector: _, paths: _, compute_checksum: _, no_index_symbols: _ }
        | cli::Command::Exclude { selector: _, paths: _ }
        | cli::Command::Experimental {  }
        | cli::Command::Lock { resolution_opts: _ }
        | cli::Command::Sync { resolution_opts: _, no_prune: _, verify: _, repair: _ }
        | cli::Command::Versions { iri: _, resolution_opts: _ }
        | cli::Command::Env { command: _ }
        | cli::Command::PrintRoot { selector: _ } => true,
        cli::Command::Init { path: _, name: _, publisher: _, version: _, license: _, metamodel: _, metamodel_release: _ }
        // Deliberately not: a discovery error would be fatal, and an
        // unlisted json project under a workspace root is exactly the
        // one a user wants to migrate. A bare `migrate` at a workspace
        // root is refused by the command itself.
        | cli::Command::Migrate { path: _, publisher: _, metamodel: _, allow_dirty: _, allow_no_vcs: _, resolution_opts: _ }
        | cli::Command::Clone { locator: _, target: _, version_constraint: _, no_deps: _, resolution_opts: _ }
        | cli::Command::Index { command: _ }
        | cli::Command::Auth { command: _ } => false,
    }
}

/// Every `sysand.toml` between `cwd` and the workspace root, which is
/// none when `root` is not a workspace root above `cwd`.
///
/// Upward only, and stopping *below* the root: the root's own file is
/// the one being used, not a stray. "there is no downward scan" -- a
/// member that is not an ancestor of `cwd` is never looked at, so a
/// workspace with fifty members does not stat fifty directories.
fn stray_configs(cwd: &Utf8Path, root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut strays = Vec::new();
    let mut current = Some(cwd);
    while let Some(dir) = current {
        if dir == root {
            return Ok(strays);
        }
        let candidate = dir.join(CONFIG_FILE);
        if wrapfs::is_file(&candidate)? {
            strays.push(candidate);
        }
        current = dir.parent();
    }
    // `root` was not an ancestor of `cwd`, so there is no workspace
    // above this directory and nothing is being ignored.
    Ok(Vec::new())
}

/// The member directory `--member` names, if it names one.
///
/// workspaces.md "Command scopes": `--member <path-or-name>` picks one
/// member, matched against its `members` entry **as written** or against
/// its project name. The rule itself is
/// [`Workspace::member`](sysand_core::workspace::model_toml::Workspace::member)
/// -- a union over both halves, refusing anything but a single answer --
/// and this only supplies it with a loaded workspace.
///
/// Loading, not consulting: a project's name is in its own manifest, so
/// a name selector cannot be answered from the table. The load is
/// therefore gated on the flag being given, and a command run without
/// it does not load the workspace.
///
/// A selector with no workspace around `cwd` is an error: dropping it
/// would act on the current project instead.
///
/// `MemberChecks::All`: a member holding its own lockfile, `.sysand` or
/// `output` is refused (workspaces.md "Stray per-member state"), and a
/// selector resolves against a workspace like anything else.
fn selected_member(
    cwd: &Utf8Path,
    workspace_root: Option<&Utf8Path>,
    selector: Selector<'_>,
) -> Result<Option<Utf8PathBuf>> {
    let Selector::Member(selector) = selector else {
        return Ok(None);
    };
    // Dropping the selector would act on the current project instead of
    // the one the user named
    let Some(root) = workspace_root else {
        bail!("`--member {selector}` needs a workspace, but `{cwd}` is not in one");
    };
    let workspace = Workspace::load_with(root, MemberChecks::All)?;
    Ok(Some(workspace.member(selector)?.dir.clone()))
}

/// Where `sysand.toml` is read and written for a command run in `cwd`.
///
/// The workspace root when there is one, the project root otherwise, and
/// the current directory when there is no project either.
///
/// `workspace_root` comes from [`discover_table`] and not from
/// [`sysand_core::workspace::model_toml::Workspace::discover`]:
/// configuration is not about the members, and a broken member must not
/// change where `sysand.toml` is read from.
///
/// A discovery *error* -- a nested workspace, an ancestor `model.toml`
/// that does not parse -- arrives here as `None` rather than as a
/// failure. The command that actually needs the workspace reports it;
/// failing configuration loading would take down every command,
/// including the ones that never look at a workspace.
fn config_root(
    cwd: &Utf8Path,
    project_root: Option<&Utf8Path>,
    workspace_root: Option<&Utf8Path>,
) -> Utf8PathBuf {
    workspace_root.or(project_root).unwrap_or(cwd).to_owned()
}

pub fn get_overrides<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    config: &Config,
    project_root: P,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<Overrides<Policy>> {
    let mut overrides = Vec::new();
    for config_project in &config.projects {
        for identifier in &config_project.identifiers {
            let mut projects = Vec::new();
            for source in &config_project.sources {
                projects.push(ProjectReference::new(AnyProject::try_from_override_source(
                    source.clone(),
                    &project_root,
                    auth_policy.clone(),
                    client.clone(),
                    runtime.clone(),
                )?));
            }
            overrides.push((
                Identifier::from_iri(&Iri::parse(identifier.as_str())?),
                projects,
            ));
        }
    }
    Ok(overrides)
}

/// Quote a string for a POSIX shell or CMD/PowerShell. CMD and
/// PowerShell have slightly different semantics, so this uses their
/// common behaviour only.
///
/// Likely to handle edge cases incorrectly, so only suitable for suggestions
pub fn quote_for_shell(arg: &str) -> String {
    #[cfg(unix)]
    {
        // original length + 2 surrounding quotes
        let mut out = String::with_capacity(arg.len() + 2);
        out.push('\'');

        for c in arg.chars() {
            if c == '\'' {
                // `\` is not allowed inside '' string, so end the string,
                // put the escaped `'` and start another string. Strings
                // without a space between them will be concatenated into a
                // single arg by the shell
                out.push_str(r"'\''");
            } else {
                out.push(c);
            }
        }

        out.push('\'');
        out
    }

    #[cfg(windows)]
    {
        use std::iter::repeat_n;

        let mut out = String::with_capacity(arg.len() + 2);
        out.push('"');

        let mut backslashes = 0;

        for c in arg.chars() {
            // Special `\` handling to work with CommandLineToArgvW
            // https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-commandlinetoargvw#remarks
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    out.extend(repeat_n('\\', backslashes * 2 + 1));
                    out.push('"');
                    backslashes = 0;
                }
                _ => {
                    out.extend(repeat_n('\\', backslashes));
                    backslashes = 0;
                    out.push(c);
                }
            }
        }

        out.extend(repeat_n('\\', backslashes * 2));
        out.push('"');
        out
    }
}

#[cfg(test)]
#[path = "./lib_tests.rs"]
mod tests;
