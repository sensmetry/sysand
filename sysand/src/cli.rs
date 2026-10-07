// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{ffi::OsStr, fmt::Display, str::FromStr as _};

use camino::Utf8PathBuf;
use clap::{ValueEnum, builder::PossibleValue, crate_authors, parser::ValueSource};
use fluent_uri::Iri;
use semver::{Version, VersionReq};
use sysand_core::{
    build::KparCompressionMethod,
    commands::{auth::IndexKey, sources::Dependencies as CoreDependencies},
    index_location::IndexLocation,
    model::{
        IndexName, IndexPublisher, KERML_SPEC_PREFIX, LICENSE_EXPRESSION_HELP, ProjectFieldError,
        ProjectName, ProjectPublisher, SYSML_SPEC_PREFIX,
    },
};

use crate::{env_vars, style::USAGE};

/// A package manager for SysML v2 and KerML
///
/// Documentation:
/// <https://docs.sysand.com/client/>
/// Package index and more information:
/// <https://sysand.com/>
/// Project repository:
/// <https://github.com/sensmetry/sysand/>
#[derive(clap::Parser, Debug)]
#[command(
    version,
    long_about,
    verbatim_doc_comment,
    arg_required_else_help = true,
    disable_help_flag = true,
    disable_version_flag = true,
    styles=crate::style::STYLING,
    author = crate_authors!(",\n"),
    help_template = "\
{before-help}{about-with-newline}
{usage-heading} {usage}

{all-args}

{name} v{version}
Developed by: {author-with-newline}
{after-help}"
)]
pub struct Args {
    #[command(flatten)]
    pub global_opts: GlobalOptions,

    #[command(subcommand)]
    pub command: Command,

    /// Display the sysand version.
    #[arg(short = 'V', long, action = clap::ArgAction::Version)]
    version: Option<bool>,
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum Command {
    /// Create a new project
    Init {
        /// The path to use for the project. Defaults to current directory
        path: Option<Utf8PathBuf>,
        /// The name of the project. Defaults to the directory name
        #[arg(long, value_parser = parse_project_name)]
        name: Option<ProjectName>,
        /// The publisher of the project. Should be the person/team/organization
        /// developing the project. It is (together with name) used
        /// to uniquely refer to a project by other projects when added as a
        /// usage (dependency)
        #[arg(long, value_parser = parse_project_publisher, verbatim_doc_comment)]
        publisher: ProjectPublisher,
        /// Set the version in SemVer 2.0 format. Defaults to `0.0.1`
        #[arg(long)]
        version: Option<Version>,
        /// Set the license in the form of an SPDX license expression.
        /// Defaults to omitting the license field
        #[arg(
            long,
            alias = "licence",
            value_name = "LICENSE",
            value_parser = parse_spdx_expression,
            verbatim_doc_comment
        )]
        license: Option<spdx::Expression>,
        /// Set a SysML v2 or KerML metamodel. Defaults to omitting the
        /// metamodel field
        #[arg(long, value_name = "KIND", value_enum, verbatim_doc_comment)]
        metamodel: Option<MetamodelKind>,
        /// Choose the release of the SysML v2 or KerML metamodel.
        /// SysML 2.0 and KerML 1.0 have the same release dates
        #[arg(
            long,
            value_name = "YYYYMMXX",
            requires = "metamodel",
            value_enum,
            verbatim_doc_comment,
            default_value = MetamodelVersion::RELEASE
        )]
        metamodel_release: MetamodelVersion,
    },
    // Only for better error messages
    #[command(hide = true)]
    New {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Add usage to project information
    Add {
        #[clap(flatten)]
        locator: AddProjectLocatorArgs,
        /// A constraint on the allowed versions of a used project.
        /// Assumes that the project being added uses Semantic Versioning.
        /// Version constraints use same syntax as Rust's Cargo.
        /// Examples: `1.2.3`, `<2`, `>=3`
        #[clap(long, verbatim_doc_comment)]
        version_constraint: Option<VersionReq>,

        #[clap(flatten)]
        sync: LockSyncPrune,

        #[command(flatten)]
        resolution_opts: ResolutionOptions,
        #[command(flatten)]
        source_opts: Box<ProjectSourceOptions>,
    },
    /// Remove usage from project information
    #[clap(alias = "rm")]
    Remove {
        #[clap(flatten)]
        locator: RemoveProjectLocatorArgs,

        #[clap(flatten)]
        sync: LockSyncPrune,

        #[command(flatten)]
        resolution_opts: ResolutionOptions,
    },
    /// Clone a project to a specified directory.
    /// Equivalent to manually downloading, extracting the
    /// project to the directory and running `sysand sync`
    #[clap(verbatim_doc_comment)]
    Clone {
        #[clap(flatten)]
        locator: CloneProjectLocatorArgs,
        /// Path to clone the project into. If already exists, must
        /// be an empty directory. Defaults to current directory
        #[arg(long, default_value = None, verbatim_doc_comment)]
        target: Option<Utf8PathBuf>,
        /// Version constraint of the project to clone; the highest
        /// matching version is chosen. A bare version such as `1.2.3`
        /// means `^1.2.3`; use `=1.2.3` for an exact version.
        /// Defaults to `*` (latest non-prerelease version)
        #[arg(long, verbatim_doc_comment)]
        version_constraint: Option<VersionReq>,

        /// Don't resolve or install dependencies
        #[arg(long)]
        no_deps: bool,
        #[command(flatten)]
        resolution_opts: ResolutionOptions,
    },
    /// Include model interchange files in project metadata. This can
    /// be used multiple times for the same file to update its metadata,
    /// as the metadata will not be updated automatically
    #[clap(verbatim_doc_comment)]
    Include {
        /// File(s) to include in the project.
        #[arg(num_args = 1..)]
        paths: Vec<Utf8PathBuf>,
        /// Compute and add each file's (current) SHA256 checksum
        // TODO: will it ever be automatically updated?
        //       Maybe only when building a kpar?
        #[arg(long)]
        compute_checksum: bool,
        /// Do not detect and add top level symbols to index
        #[arg(long)]
        no_index_symbols: bool,
    },
    /// Exclude model interchange file from project metadata
    Exclude {
        /// File(s) to exclude from the project
        #[arg(num_args = 1..)]
        paths: Vec<Utf8PathBuf>,
    },
    /// Build a KerML Project Archive (KPAR). If executed in a workspace
    /// outside of a project, builds all projects in the workspace.
    #[clap(verbatim_doc_comment)]
    Build {
        /// Path for the finished KPAR or KPARs. When building a
        /// workspace, it is a path to the folder to write the KPARs to
        /// (default: `<current-workspace>/output`). When building a single
        /// project, it is a path to the KPAR file to write (default
        /// `<current-workspace>/output/<project name>-<version>.kpar` or
        /// `<current-project>/output/<project name>-<version>.kpar` depending
        /// on whether the current project belongs to a workspace or not).
        #[clap(verbatim_doc_comment)]
        path: Option<Utf8PathBuf>,
        /// Method to compress the files in the KPAR
        #[arg(long, default_value_t, value_enum)]
        compression: KparCompressionMethodCli,
        /// Allow usages of local paths (`file://`).
        /// Warning: using this makes the project not portable between different
        /// computers, as `file://` URL always contains an absolute path.
        /// For multiple related projects, consider using a workspace instead
        #[arg(long, verbatim_doc_comment)]
        allow_path_usage: bool,
        /// Don't update exported symbols index in the built KPAR metadata
        #[arg(long)]
        keep_index: bool,
    },
    /// Publish a KPAR to a sysand package index
    Publish {
        /// Path to the KPAR file to publish. If not provided, will look
        /// for a KPAR in the output directory with the current project's
        /// name and version
        #[clap(verbatim_doc_comment)]
        path: Option<Utf8PathBuf>,

        /// Configured index URL to publish to (e.g. https://sysand.com)
        /// The index must advertise a publish endpoint: its
        /// sysand-index-config.json must set `api_root`
        #[arg(long, value_name = "URL", verbatim_doc_comment, value_parser = IndexLocationParser)]
        index: IndexLocation,

        /// How to use CI trusted publishing for acquiring publish credentials
        #[arg(
            long,
            value_enum,
            value_name = "MODE",
            default_value = "auto",
            verbatim_doc_comment
        )]
        trusted_publishing: TrustedPublishingMode,
    },
    /// Manage stored credentials for package indexes
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Create or update lockfile
    Lock {
        #[command(flatten)]
        resolution_opts: ResolutionOptions,
    },
    /// Create a local `.sysand` directory for installing dependencies
    Env {
        #[command(subcommand)]
        command: Option<EnvCommand>,
    },
    /// Manage a local sysand index
    Index {
        #[command(subcommand)]
        command: IndexCommand,
    },
    /// Sync `.sysand` to lockfile, creating a lockfile and `.sysand` if needed
    Sync {
        #[command(flatten)]
        resolution_opts: ResolutionOptions,
        /// Don't remove projects that are no longer needed from `.sysand`
        #[arg(long)]
        no_prune: bool,
    },
    /// Describe a local project (either the current one or one at a
    /// given path) or resolve and describe a project at a specified
    /// IRI/URL. Use `sysand edit` to modify a local project
    #[clap(verbatim_doc_comment)]
    Info {
        #[clap(flatten)]
        locator: InfoProjectLocatorArgs,
        /// Print only the value of the given field (a list field one entry
        /// per line). Prints nothing if the field is not set
        #[arg(long, value_name = "FIELD", value_enum, verbatim_doc_comment)]
        get: Option<InfoField>,
        // TODO: is this useful?
        // /// Do not try to normalise the IRI/URI when resolving
        // #[arg(long, visible_alias = "no-normalize")]
        // no_normalise: bool,
        // TODO: Add various options, such as whether to take local environment
        //       into consideration
        #[command(flatten)]
        resolution_opts: ResolutionOptions,
    },
    /// Modify the information (`.project.json`) and metadata (`.meta.json`)
    /// of a local project: the current one, or the one in `--dir`.
    /// Several fields can be edited at once
    #[clap(verbatim_doc_comment, arg_required_else_help = true)]
    Edit {
        /// Edit the project in the given directory instead of the current
        /// project
        #[arg(long, verbatim_doc_comment)]
        dir: Option<Utf8PathBuf>,
        #[command(flatten)]
        edits: Box<EditArgs>,
    },
    /// List source files for the current project and (optionally)
    /// its dependencies available in `.sysand`. Requires that
    /// `.sysand` is up to date, so it's recommended to run
    /// `sysand sync` prior to this
    #[clap(verbatim_doc_comment)]
    Sources {
        #[command(flatten)]
        sources_opts: SourcesOptions,
    },
    /// Prints the root directory of the current project
    PrintRoot,
    /// Experimental commands. Likely to change in incompatible ways or be
    /// removed in the future. Currently no experimental commands are included
    #[clap(hide = true, verbatim_doc_comment)]
    Experimental {
        // Kept as a placeholder only for future experimental commands
    },
}

#[derive(clap::Args, Debug, Clone)]
#[group(required = false, multiple = true)]
pub struct LockSyncPrune {
    // TODO: consider enforcing the implication here via e.g. default_value_if(s);
    // the issue is that it does not work transitively and is verbose. Alternatively,
    // do so in lib.rs with e.g. fn imply(bools: [&mut bool]), where the first
    // implies second implies third and so on
    /// Do not automatically resolve dependencies (and generate/update
    /// the lockfile). Implies `--no-sync`
    #[arg(long, verbatim_doc_comment)]
    pub no_lock: bool,
    /// Do not automatically install/update/remove dependencies. Implies
    /// `--no-prune`
    #[arg(long, verbatim_doc_comment)]
    pub no_sync: bool,
    /// Don't remove projects that are no longer needed from `.sysand`
    #[arg(long)]
    pub no_prune: bool,
}

#[derive(clap::Args, Debug, Clone)]
#[group(required = true, multiple = false)]
pub struct AddProjectLocatorArgs {
    /// Project identifier of the form `<publisher>/<name>`, added as an
    /// index usage. `<publisher>` and `<name>` can either exactly match
    /// those of the project being added (e.g. `"Acme Labs/My Lib"`), or
    /// use lowercase letters only and replace spaces with `-` (e.g.
    /// `acme-labs/my-lib`) to take the project's spelling.
    /// With `--no-lock`, the spelling is checked against, or taken from,
    /// the versions installed in the local environment
    #[clap(
        default_value = None,
        value_name = "IDENTIFIER",
        value_parser = with_tip(
            parse_index_identifier,
            "to add from a directory, a KPAR or an IRI, use `--dir`, `--kpar-path` or `--iri` respectively"
        ),
        verbatim_doc_comment,
        // conflict with iri/iri_path is currently a no-op, as multiple=false;
        // this is for the future when `--identifier` will be usable with
        // `--dir` and other types, but not `--iri`
        conflicts_with_all = ["source", "iri", "iri_path"]
    )]
    pub identifier: Option<(IndexPublisher, IndexName)>,
    /// Add a project from a given directory path
    #[arg(long, verbatim_doc_comment,
        conflicts_with_all = ["source", "iri", "iri_path", "version_constraint"])]
    pub dir: Option<Utf8PathBuf>,
    /// Add a project from a KPAR at a given path
    #[arg(long, verbatim_doc_comment,
        conflicts_with_all = ["source", "iri", "iri_path", "version_constraint"])]
    pub kpar_path: Option<Utf8PathBuf>,
    /// IRI/URI/URL identifying the project to be used. Use `--iri-path`
    /// for paths.
    /// Where possible, consider using `--dir` or `--kpar-path`. They
    /// explicitly identify the project separately from how it is
    /// obtained
    #[clap(
        long,
        default_value = None,
        value_parser = with_tip(
            Iri::from_str,
            "if you wanted to use a path, use `--dir`, `--kpar-path` or `--iri-path` instead; \
             for a project `<publisher>/<name>`, pass it without `--iri`"
        ),
        verbatim_doc_comment
    )]
    pub iri: Option<Iri<String>>,
    /// Path to the project to be added, after converting to a `file://` IRI.
    ///
    /// Deprecation notice: using this flag makes the project not portable
    /// between different computers, as `file://` URL always contains an
    /// absolute path. Use `--dir` or `--kpar-path` to record a relative
    /// path instead.
    /// Currently, this flag is still necessary when the usage is on another
    /// drive (on Windows only) because this flag allows pointing to it, unlike
    /// `--dir`/`--kpar-path`, which currently forbid absolute paths
    #[arg(
        long,
        default_value = None,
        verbatim_doc_comment,
        conflicts_with = "iri"
    )]
    pub iri_path: Option<Utf8PathBuf>,
}

#[derive(clap::Args, Debug, Clone)]
#[group(required = true, multiple = false)]
pub struct RemoveProjectLocatorArgs {
    /// Project identifier of the form `<publisher>/<name>`. `<publisher>`
    /// and `<name>` can either exactly match those of the project being
    /// removed, or be normalized: lowercase, with each run of spaces and
    /// punctuation replaced by `-` (a single `.` is kept) and removed at
    /// either end
    #[clap(
        default_value = None,
        value_name = "IDENTIFIER",
        value_parser = with_tip(
            parse_project_identifier,
            "to remove a usage by IRI, use `--iri` or `--iri-path`"
        ),
        verbatim_doc_comment
    )]
    pub identifier: Option<(ProjectPublisher, ProjectName)>,
    /// IRI identifying the project usage to be removed. Use `--iri-path`
    /// for paths
    #[clap(
        long,
        default_value = None,
        value_parser = with_tip(
            Iri::from_str,
            "if you wanted to use a path, use `--iri-path` instead; \
             for a project `<publisher>/<name>`, pass it without `--iri`"
        ),
        verbatim_doc_comment
    )]
    pub iri: Option<Iri<String>>,
    /// Path to the project to be removed from usages after conversion to a
    /// `file://` IRI
    #[arg(
        long,
        default_value = None,
        verbatim_doc_comment
    )]
    pub iri_path: Option<Utf8PathBuf>,
}

#[derive(clap::Args, Debug, Clone)]
#[group(required = true, multiple = false)]
pub struct CloneProjectLocatorArgs {
    /// Project identifier of the form `<publisher>/<name>`. `<publisher>`
    /// and `<name>` can either exactly match those of the project being
    /// cloned, or use lowercase letters only and replace spaces with `-`.
    /// The project is resolved as an index usage
    #[clap(
        default_value = None,
        value_name = "IDENTIFIER",
        value_parser = with_tip(
            parse_index_identifier,
            "to clone from a directory, a KPAR or an IRI, use `--dir`, `--kpar-path` or `--iri` respectively"
        ),
        verbatim_doc_comment
    )]
    pub identifier: Option<(IndexPublisher, IndexName)>,
    /// Clone a project from a given directory path
    #[arg(long, verbatim_doc_comment)]
    pub dir: Option<Utf8PathBuf>,
    /// Clone a project from a KPAR at a given path
    #[arg(long, verbatim_doc_comment)]
    pub kpar_path: Option<Utf8PathBuf>,
    /// IRI/URI/URL identifying the project to be cloned. Use `--dir` or
    /// `--kpar-path` for paths
    #[arg(
        long,
        default_value = None,
        value_parser = with_tip(
            Iri::from_str,
            "if you wanted to use a path, use `--dir` or `--kpar-path` instead"
        ),
        verbatim_doc_comment
    )]
    pub iri: Option<Iri<String>>,
}

/// Selects the project to describe or modify. If none is given, the
/// current project is used
#[derive(clap::Args, Debug, Clone)]
#[group(required = false, multiple = false)]
pub struct InfoProjectLocatorArgs {
    /// Project identifier of the form `<publisher>/<name>`. `<publisher>`
    /// and `<name>` can either exactly match those of the project being
    /// described, or use lowercase letters only and replace spaces with `-`.
    /// The project is resolved as an index usage, and its highest version
    /// is described
    #[clap(
        default_value = None,
        value_name = "IDENTIFIER",
        value_parser = with_tip(
            parse_index_identifier,
            "to use a directory, a KPAR or an IRI, use `--dir`, `--kpar-path` or `--iri`; \
            to print a single field, use `--get <FIELD>`"
        ),
        verbatim_doc_comment
    )]
    pub identifier: Option<(IndexPublisher, IndexName)>,
    /// Use the project in a given directory instead of the current project.
    #[arg(long, verbatim_doc_comment)]
    pub dir: Option<Utf8PathBuf>,
    /// Use the project from a KPAR at a given path instead of the current
    /// project
    #[arg(long, verbatim_doc_comment)]
    pub kpar_path: Option<Utf8PathBuf>,
    /// IRI/URI/URL identifying the project to be resolved and used instead
    /// of the current project. Use `--dir` or `--kpar-path` for paths
    #[arg(
        long,
        default_value = None,
        value_parser = with_tip(
            Iri::from_str,
            "if you wanted to use a path, use `--dir` or `--kpar-path` instead"
        ),
        verbatim_doc_comment
    )]
    pub iri: Option<Iri<String>>,
}

/// Selects a project installed in `.sysand`
#[derive(clap::Args, Debug, Clone)]
#[group(required = true, multiple = false)]
pub struct EnvProjectLocatorArgs {
    /// Project identifier of the form `<publisher>/<name>`. `<publisher>`
    /// and `<name>` can either exactly match those of the installed
    /// project, or be normalized: lowercase, with each run of spaces and
    /// punctuation replaced by `-` (a single `.` is kept) and removed at
    /// either end
    #[clap(
        default_value = None,
        value_name = "IDENTIFIER",
        value_parser = with_tip(parse_project_identifier, "to use an IRI, use `--iri`"),
        verbatim_doc_comment
    )]
    pub identifier: Option<(ProjectPublisher, ProjectName)>,
    /// IRI identifying the installed project
    #[arg(long, default_value = None)]
    pub iri: Option<Iri<String>>,
}

#[derive(clap::ValueEnum, Default, Copy, Clone, Debug)]
#[clap(rename_all = "lowercase")]
pub enum KparCompressionMethodCli {
    /// Store the files as is
    Stored,
    /// Compress the files using Deflate
    #[default]
    Deflated,
    /// Compress the files using BZIP2
    #[cfg(feature = "kpar-bzip2")]
    Bzip2,
    /// Compress the files using ZStandard
    #[cfg(feature = "kpar-zstd")]
    Zstd,
    /// Compress the files using XZ
    #[cfg(feature = "kpar-xz")]
    Xz,
    /// Compress the files using PPMd
    #[cfg(feature = "kpar-ppmd")]
    Ppmd,
}

impl From<KparCompressionMethodCli> for KparCompressionMethod {
    fn from(value: KparCompressionMethodCli) -> Self {
        match value {
            KparCompressionMethodCli::Stored => Self::Stored,
            KparCompressionMethodCli::Deflated => Self::Deflated,
            #[cfg(feature = "kpar-bzip2")]
            KparCompressionMethodCli::Bzip2 => Self::Bzip2,
            #[cfg(feature = "kpar-zstd")]
            KparCompressionMethodCli::Zstd => Self::Zstd,
            #[cfg(feature = "kpar-xz")]
            KparCompressionMethodCli::Xz => Self::Xz,
            #[cfg(feature = "kpar-ppmd")]
            KparCompressionMethodCli::Ppmd => Self::Ppmd,
        }
    }
}

#[derive(clap::ValueEnum, Copy, Clone, Debug, PartialEq, Eq)]
#[clap(rename_all = "lowercase")]
pub enum TrustedPublishingMode {
    /// Use trusted publishing in supported CI environments
    Auto,
    /// Require trusted publishing and fail outside supported CI environments
    Always,
    /// Disable trusted publishing and require explicitly configured publish credentials
    #[value(alias = "false")]
    Never,
}

impl From<TrustedPublishingMode> for sysand_core::commands::publish::TrustedPublishingMode {
    fn from(value: TrustedPublishingMode) -> Self {
        match value {
            TrustedPublishingMode::Auto => Self::Auto,
            TrustedPublishingMode::Always => Self::Always,
            TrustedPublishingMode::Never => Self::Never,
        }
    }
}

// This is implemented mainly so that if KparCompressionMethod gets a new member
// and KparCompressionMethodCli isn't updated it would give a compilation error
impl From<KparCompressionMethod> for KparCompressionMethodCli {
    fn from(value: KparCompressionMethod) -> Self {
        match value {
            KparCompressionMethod::Stored => Self::Stored,
            KparCompressionMethod::Deflated => Self::Deflated,
            #[cfg(feature = "kpar-bzip2")]
            KparCompressionMethod::Bzip2 => Self::Bzip2,
            #[cfg(feature = "kpar-zstd")]
            KparCompressionMethod::Zstd => Self::Zstd,
            #[cfg(feature = "kpar-xz")]
            KparCompressionMethod::Xz => Self::Xz,
            #[cfg(feature = "kpar-ppmd")]
            KparCompressionMethod::Ppmd => Self::Ppmd,
        }
    }
}

/// Parse an index URL argument with `parse`. Unlike clap's default
/// error, the one returned does not echo the raw value, which may contain
/// credentials (the parse error names the URL with userinfo redacted), and
/// it names the environment variable rather than the option when the value
/// came from one (clap does not, see
/// <https://github.com/clap-rs/clap/issues/5202>).
fn parse_index_value<T, E: Display>(
    cmd: &clap::Command,
    arg: Option<&clap::Arg>,
    value: &OsStr,
    source: ValueSource,
    parse: impl FnOnce(&str) -> Result<T, E>,
) -> Result<T, clap::Error> {
    let value = value
        .to_str()
        .ok_or_else(|| clap::Error::new(clap::error::ErrorKind::InvalidUtf8).with_cmd(cmd))?;
    parse(value).map_err(|e| {
        let env = arg
            .and_then(clap::Arg::get_env)
            .filter(|_| source == ValueSource::EnvVariable);
        let mut message = if let Some(env) = env {
            format!("invalid `{}` environment variable: {e}", env.display())
        } else {
            let name = arg.map_or_else(|| "...".to_owned(), ToString::to_string);
            format!("invalid value for '{name}': {e}")
        };
        // An empty value most likely comes from a stray delimiter or an
        // empty environment variable
        if value.is_empty() {
            let delimiter = arg.and_then(clap::Arg::get_value_delimiter);
            let hint = match (delimiter, env) {
                (Some(d), Some(env)) => Some(format!(
                    "check for a stray `{d}`, or for `{}` set to an empty value",
                    env.display()
                )),
                (Some(d), None) => Some(format!("check for a stray `{d}`")),
                (None, Some(env)) => Some(format!(
                    "check for `{}` set to an empty value",
                    env.display()
                )),
                (None, None) => None,
            };
            if let Some(hint) = hint {
                message.push_str("\nhint: ");
                message.push_str(&hint);
            }
        }
        message.push('\n');
        clap::Error::raw(clap::error::ErrorKind::ValueValidation, message).with_cmd(cmd)
    })
}

/// Parses an [`IndexLocation`], see [`parse_index_value`].
#[derive(Clone, Debug)]
pub struct IndexLocationParser;

impl clap::builder::TypedValueParser for IndexLocationParser {
    type Value = IndexLocation;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        self.parse_ref_(cmd, arg, value, ValueSource::CommandLine)
    }

    fn parse_ref_(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
        source: ValueSource,
    ) -> Result<Self::Value, clap::Error> {
        parse_index_value(cmd, arg, value, source, IndexLocation::parse)
    }
}

/// Parses an [`IndexKey`] (with the credential-specific errors of
/// [`IndexKey::validate`]), see [`parse_index_value`]
#[derive(Clone, Debug)]
struct IndexKeyParser;

impl clap::builder::TypedValueParser for IndexKeyParser {
    type Value = IndexKey;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        self.parse_ref_(cmd, arg, value, ValueSource::CommandLine)
    }

    fn parse_ref_(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
        source: ValueSource,
    ) -> Result<Self::Value, clap::Error> {
        parse_index_value(cmd, arg, value, source, IndexKey::validate)
    }
}

/// Wrap `inner` value parser, adding `tip` to its errors. The tip is
/// rendered by clap after the error message
fn with_tip<P>(inner: P, tip: &'static str) -> WithTip<P> {
    WithTip { inner, tip }
}

/// See [`with_tip`]
#[derive(Clone, Debug)]
struct WithTip<P> {
    inner: P,
    tip: &'static str,
}

impl<P: clap::builder::TypedValueParser> clap::builder::TypedValueParser for WithTip<P> {
    type Value = P::Value;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        self.inner.parse_ref(cmd, arg, value).map_err(|mut e| {
            e.insert(
                clap::error::ContextKind::Suggested,
                clap::error::ContextValue::StyledStrs(vec![self.tip.into()]),
            );
            e
        })
    }
}

/// A field of the project information (`.project.json`) or metadata
/// (`.meta.json`)
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoField {
    /// Project name
    Name,
    /// Project publisher
    Publisher,
    /// Project description
    Description,
    /// Project version
    Version,
    /// Project license (SPDX license expression)
    #[value(alias = "licence")]
    License,
    /// Project maintainers, one per line
    Maintainer,
    /// Project website
    Website,
    /// Project topics, one per line
    Topic,
    /// Project usages (dependencies), one per line
    Usage,
    /// Symbols exported by the project, one per line
    Index,
    /// Creation time of the project metadata
    Created,
    /// Metamodel of the project
    Metamodel,
    /// Whether the project includes derived properties
    IncludesDerived,
    /// Whether the project includes implied properties
    IncludesImplied,
    /// Checksums of the project source files, one per line
    Checksum,
}

/// Edits of a local project. Edits of different fields can be combined;
/// the project information and metadata are each written once
#[derive(clap::Args, Debug, Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each `--clear-*` flag is an independent CLI switch"
)]
#[group(required = true, multiple = true)]
pub struct EditArgs {
    /// Set the name
    #[arg(long, value_parser = parse_project_name, help_heading = "Project information")]
    pub name: Option<ProjectName>,
    /// Set the publisher
    #[arg(long, value_parser = parse_project_publisher, help_heading = "Project information")]
    pub publisher: Option<ProjectPublisher>,
    /// Set the version in SemVer 2.0 format
    #[arg(long, help_heading = "Project information")]
    pub version: Option<Version>,
    /// Set the description
    #[arg(
        long,
        value_name = "TEXT",
        conflicts_with = "clear_description",
        help_heading = "Project information"
    )]
    pub description: Option<String>,
    /// Remove the description
    #[arg(long, help_heading = "Project information")]
    pub clear_description: bool,
    /// Set the license in the form of an SPDX license expression
    #[arg(
        long,
        alias = "licence",
        value_name = "LICENSE",
        value_parser = parse_spdx_expression,
        conflicts_with = "clear_license",
        help_heading = "Project information"
    )]
    pub license: Option<spdx::Expression>,
    /// Remove the license
    #[arg(long, alias = "clear-licence", help_heading = "Project information")]
    pub clear_license: bool,
    /// Set the website. Must be a valid IRI/URI/URL; `https://` is assumed
    /// if it has no scheme
    #[arg(
        long,
        value_name = "URI",
        value_parser = parse_https_iri,
        conflicts_with = "clear_website",
        verbatim_doc_comment,
        help_heading = "Project information"
    )]
    pub website: Option<Iri<String>>,
    /// Remove the website
    #[arg(long, help_heading = "Project information")]
    pub clear_website: bool,
    /// Add a maintainer. Can be repeated
    #[arg(long, value_name = "MAINTAINER", help_heading = "Project information")]
    pub add_maintainer: Vec<String>,
    /// Remove a maintainer (every entry equal to the value). Can be repeated
    #[arg(long, value_name = "MAINTAINER", help_heading = "Project information")]
    pub remove_maintainer: Vec<String>,
    /// Remove all maintainers. Applied before `--add-maintainer`, so the two
    /// together replace the maintainers
    #[arg(
        long,
        conflicts_with = "remove_maintainer",
        verbatim_doc_comment,
        help_heading = "Project information"
    )]
    pub clear_maintainers: bool,
    /// Add a topic. Can be repeated
    #[arg(long, value_name = "TOPIC", help_heading = "Project information")]
    pub add_topic: Vec<String>,
    /// Remove a topic (every entry equal to the value). Can be repeated
    #[arg(long, value_name = "TOPIC", help_heading = "Project information")]
    pub remove_topic: Vec<String>,
    /// Remove all topics. Applied before `--add-topic`, so the two together
    /// replace the topics
    #[arg(
        long,
        conflicts_with = "remove_topic",
        verbatim_doc_comment,
        help_heading = "Project information"
    )]
    pub clear_topics: bool,
    // It would be nicer to have Option<Metamodel> here,
    // but that would introduce an additional level of
    // nesting, as clap does not support flatten with Option
    /// Set a SysML v2 or KerML metamodel. To set a custom metamodel, use
    /// `--custom-metamodel`
    #[arg(
        long,
        value_name = "KIND",
        value_enum,
        verbatim_doc_comment,
        help_heading = "Project metadata"
    )]
    pub metamodel: Option<MetamodelKind>,
    /// Choose the release of the SysML v2 or KerML metamodel.
    /// SysML 2.0 and KerML 1.0 have the same release dates
    #[arg(
        long,
        value_name = "YYYYMMXX",
        requires = "metamodel",
        value_enum,
        verbatim_doc_comment,
        default_value = MetamodelVersion::RELEASE,
        help_heading = "Project metadata"
    )]
    pub metamodel_release: MetamodelVersion,
    /// Choose a custom release of the SysML v2 or KerML metamodel
    #[arg(
        long,
        value_name = "YYYYMMXX",
        requires = "metamodel",
        conflicts_with = "metamodel_release",
        help_heading = "Project metadata"
    )]
    pub metamodel_release_custom: Option<u32>,
    /// Set a custom metamodel. To set a SysML v2 or KerML metamodel, use
    /// `--metamodel`
    #[arg(
        long,
        value_name = "METAMODEL",
        conflicts_with_all = ["metamodel", "metamodel_release", "metamodel_release_custom"],
        verbatim_doc_comment,
        help_heading = "Project metadata"
    )]
    pub custom_metamodel: Option<String>,
    /// Remove the metamodel
    #[arg(
        long,
        conflicts_with_all = ["metamodel", "custom_metamodel"],
        help_heading = "Project metadata"
    )]
    pub clear_metamodel: bool,
    /// Set whether the project includes derived properties
    #[arg(
        long,
        value_name = "BOOL",
        conflicts_with = "clear_includes_derived",
        help_heading = "Project metadata"
    )]
    pub includes_derived: Option<bool>,
    /// Remove whether the project includes derived properties
    #[arg(long, help_heading = "Project metadata")]
    pub clear_includes_derived: bool,
    /// Set whether the project includes implied properties
    #[arg(
        long,
        value_name = "BOOL",
        conflicts_with = "clear_includes_implied",
        help_heading = "Project metadata"
    )]
    pub includes_implied: Option<bool>,
    /// Remove whether the project includes implied properties
    #[arg(long, help_heading = "Project metadata")]
    pub clear_includes_implied: bool,
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum AuthCommand {
    /// Show the credentials sysand will authenticate with: stored index
    /// logins and `SYSAND_CRED_*` environment credentials, marking the
    /// entries that apply to the default index. For stored logins,
    /// `validated (read, api)` names the index endpoints that accepted
    /// the token at login; `not validated` means no endpoint exercised
    /// it. Never shows secrets
    #[clap(verbatim_doc_comment)]
    Status,
    /// Store a bearer token for an index. The token is read from a hidden
    /// prompt, or from standard input with `--token-stdin`, never from a
    /// command-line argument. The token is validated against the index
    /// before it is stored; a token the index rejects is not stored
    #[clap(verbatim_doc_comment)]
    Login {
        /// Index URL to log in to (e.g. https://sysand.com).
        /// URL templates are accepted (see `--index` in `sysand add
        /// --help`); the credential is scoped to the template's literal
        /// prefix. Defaults to the default index
        #[arg(verbatim_doc_comment, value_parser = IndexKeyParser)]
        index_url: Option<IndexKey>,
        /// Read the token from standard input (trimming one trailing
        /// newline) instead of prompting
        #[arg(long, verbatim_doc_comment)]
        token_stdin: bool,
    },
    /// Show who the index API identifies you as: sends one authenticated
    /// request to the index API (`v1/whoami`) with the credential sysand
    /// would use (`SYSAND_CRED_*` environment credentials take precedence
    /// over stored credentials). Query-only: nothing is stored or refreshed
    #[clap(verbatim_doc_comment)]
    Whoami {
        /// Index URL to query (e.g. https://sysand.com).
        /// URL templates are accepted (see `--index` in `sysand add
        /// --help`); the index must advertise an API in its discovery
        /// configuration. Defaults to the default index
        #[arg(verbatim_doc_comment, value_parser = IndexKeyParser)]
        index_url: Option<IndexKey>,
    },
    /// Remove a stored index login
    Logout {
        /// Index URL to log out from (e.g. https://sysand.com).
        /// URL templates are accepted (see `--index` in `sysand add
        /// --help`). Defaults to the default index
        #[arg(verbatim_doc_comment, value_parser = IndexKeyParser)]
        index_url: Option<IndexKey>,
    },
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum EnvCommand {
    // TODO: decide whether to enable or remove these commands
    // /// Install project in `.sysand`
    // Install {
    //     /// IRI identifying the project to be installed
    //     iri: fluent_uri::Iri<String>,
    //     /// Version to be installed. Defaults to the latest version
    //     /// according to SemVer 2.0; for `pkg:sysand` projects
    //     /// pre-releases are ignored unless this names one
    //     #[clap(verbatim_doc_comment)]
    //     version: Option<String>,
    //     /// Path to interchange project
    //     #[arg(long, default_value = None)]
    //     path: Option<Utf8PathBuf>,

    //     #[command(flatten)]
    //     install_opts: InstallOptions,
    //     #[command(flatten)]
    //     resolution_opts: ResolutionOptions,
    // },
    // /// Uninstall project in `.sysand`
    // Uninstall {
    //     /// IRI identifying the project to be uninstalled
    //     iri: fluent_uri::Iri<String>,
    //     /// Version to be uninstalled
    //     version: Option<String>,
    // },
    /// List projects installed in `.sysand`
    List,
    /// List source files for an installed project and
    /// (optionally) its dependencies
    #[clap(verbatim_doc_comment)]
    Sources {
        #[clap(flatten)]
        locator: Box<EnvProjectLocatorArgs>,
        /// Version constraint selecting the installed project to list
        /// sources for. A bare version such as `1.2.3` means `^1.2.3`;
        /// use `=1.2.3` for an exact version. Defaults to the first
        /// installed version found
        #[arg(long, verbatim_doc_comment)]
        version_constraint: Option<VersionReq>,

        #[command(flatten)]
        sources_opts: SourcesOptions,
    },
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum IndexCommand {
    /// Create a local sysand index
    #[clap(verbatim_doc_comment)]
    Init {
        /// Path to the index directory. If not provided, current working directory is used.
        /// If the directory does not exist, it is created
        #[arg(long, verbatim_doc_comment)]
        index_root: Option<Utf8PathBuf>,
    },
    /// Add a KPAR to a local sysand index
    #[clap(verbatim_doc_comment)]
    Add {
        // The type is String, not Iri so that a better error can be reported in some cases
        // for example when the publisher contains a space
        /// IRI identifying the project. Default is `pkg:sysand/<publisher>/<name>`, if
        /// publisher is specified in .project.json. Omitting both publisher and IRI is an error
        #[arg(long, verbatim_doc_comment)]
        iri: Option<String>,
        /// Path to KPAR
        #[arg(long, verbatim_doc_comment)]
        kpar_path: Utf8PathBuf,
        /// Path to the index directory. If not provided, current working directory is used
        #[arg(long)]
        index_root: Option<Utf8PathBuf>,
    },
    /// Yank a project version from a local index. The yanked version will still be available
    /// and used to sync from an existing lockfile, but new lockfiles will not use it.
    /// A yanked version cannot be un-yanked
    #[clap(verbatim_doc_comment)]
    Yank {
        /// IRI identifying the project
        #[arg(long)]
        iri: String,
        // It's String and not semver::Version because it's good to allow yanking a non-semantic
        // version
        /// Version to yank
        #[arg(long)]
        version: String,
        /// Path to the index directory. If not provided, current working directory is used
        #[arg(long)]
        index_root: Option<Utf8PathBuf>,
    },
    /// Remove a project or a specific version of a project from a local sysand index.
    /// This breaks the existing lockfiles which use the to-be-removed project or version.
    /// Instead it is recommended to yank a specific version and release a new fixed version.
    /// Project or version removal cannot be undone
    #[clap(verbatim_doc_comment)]
    Remove {
        /// IRI identifying the project
        #[arg(long)]
        iri: String,
        #[clap(flatten)]
        target: IndexRemoveTarget,
        /// Path to the index directory. If not provided, current working directory is used
        #[arg(long)]
        index_root: Option<Utf8PathBuf>,
    },
}

#[derive(clap::Args, Debug, Clone)]
#[group(required = true, multiple = false)]
pub struct IndexRemoveTarget {
    // It's String and not semver::Version because it's good to allow removing a non-semantic
    // version
    /// Version to remove
    #[arg(long)]
    pub version: Option<String>,
    /// Remove the whole project
    #[arg(long)]
    pub project: bool,
}

#[derive(clap::Args, Debug, Clone)]
#[group(required = false, multiple = true)]
pub struct InstallOptions {
    /// Allow overwriting existing installation
    #[arg(long)]
    pub allow_overwrite: bool,
    /// Install even if another version is already installed
    #[arg(long)]
    pub allow_multiple: bool,
    /// Don't install any dependencies
    #[arg(long)]
    pub no_deps: bool,
}

/// Control how packages and their dependencies are resolved.
/// `include_std` is here only for convenience, as it does not
/// affect package resolution, only installation
/// (in `sync`, `env install`, `lock`, etc.)
#[derive(clap::Args, Debug, Clone)]
#[group(required = false, multiple = true)]
pub struct ResolutionOptions {
    /// Comma-delimited list of index URLs to use when resolving
    /// project(s) and/or their dependencies, in addition to the default indexes.
    /// An index URL may be a URL template with a `{path}` or `{path_raw}`
    /// placeholder. `{path}` is replaced by the percent-encoded relative index
    /// path (`/` becomes `%2F`), e.g. for the GitLab repository files API:
    /// `https://gitlab.com/api/v4/projects/123/repository/files/{path}/raw?ref=main`
    /// `{path_raw}` keeps `/` literal, for hosts that accept ordinary path
    /// segments but need a suffix or query string after the path.
    #[arg(
        long,
        num_args = 0..,
        global = true,
        help_heading = "Resolution options",
        env = env_vars::SYSAND_INDEX,
        value_delimiter = ',',
        value_parser = IndexLocationParser,
        verbatim_doc_comment
    )]
    pub index: Vec<IndexLocation>,
    /// Comma-delimited list of URLs to use as default index
    /// URLs. Default indexes are tried after other indexes
    /// (default `https://sysand.com`). Accepts URL templates like --index.
    #[arg(
        long,
        num_args = 0..,
        global = true,
        help_heading = "Resolution options",
        env = env_vars::SYSAND_DEFAULT_INDEX,
        value_delimiter = ',',
        value_parser = IndexLocationParser,
        verbatim_doc_comment
    )]
    pub default_index: Vec<IndexLocation>,
    /// Do not use any index when resolving project(s) and/or their dependencies
    // TODO: document somewhere which sources are supported:
    // - file:// (sometimes also regular paths)
    // - git (https?, ssh?)
    // - http(s)
    // - index
    #[arg(
        long,
        conflicts_with_all = ["index", "default_index"],
        global = true,
        help_heading = "Resolution options",
    )]
    pub no_index: bool,
    /// Don't ignore KerML/SysML v2 standard libraries if specified as dependencies
    #[arg(long, global = true, help_heading = "Resolution options")]
    pub include_std: bool,
}

#[derive(clap::Args, Debug, Clone, Default)]
pub struct ProjectSourceOptions {
    /// Add usage as a local interchange project at PATH and
    /// update configuration file attempting to guess the
    /// source from the PATH
    #[arg(long, value_name = "PATH", group = "source", verbatim_doc_comment)]
    pub from_path: Option<Utf8PathBuf>,
    /// Add usage as a remote interchange project at URL and
    /// update configuration file attempting to guess the
    /// source from the URL
    #[arg(long, value_name = "URL", group = "source", verbatim_doc_comment)]
    pub from_url: Option<Iri<String>>,
    /// Add usage as an editable interchange project at PATH and
    /// update configuration file with appropriate source
    #[arg(long, value_name = "PATH", group = "source", verbatim_doc_comment)]
    pub as_editable: Option<Utf8PathBuf>,
    /// Add usage as a local interchange project at PATH and
    /// update configuration file with appropriate source
    #[arg(long, value_name = "PATH", group = "source", verbatim_doc_comment)]
    pub as_local_src: Option<Utf8PathBuf>,
    /// Add usage as a local interchange project archive at PATH
    /// and update configuration file with appropriate source
    #[arg(long, value_name = "PATH", group = "source", verbatim_doc_comment)]
    pub as_local_kpar: Option<Utf8PathBuf>,
    /// Add usage as a remote interchange project at URL and
    /// update configuration file with appropriate source
    #[arg(long, value_name = "URL", group = "source", verbatim_doc_comment)]
    pub as_remote_src: Option<Iri<String>>,
    /// Add usage as a remote interchange project archive at URL
    /// and update configuration file with appropriate source
    #[arg(long, value_name = "URL", group = "source", verbatim_doc_comment)]
    pub as_remote_kpar: Option<Iri<String>>,
    /// Add usage as a remote git interchange project at URL and
    /// update configuration file with appropriate source
    #[arg(long, value_name = "URL", group = "source", verbatim_doc_comment)]
    pub as_remote_git: Option<Iri<String>>,
}

impl ProjectSourceOptions {
    /// Whether a source is given, which `add` writes to the configuration file
    pub fn is_given(&self) -> bool {
        let Self {
            from_path,
            from_url,
            as_editable,
            as_local_src,
            as_local_kpar,
            as_remote_src,
            as_remote_kpar,
            as_remote_git,
        } = self;
        from_path.is_some()
            || from_url.is_some()
            || as_editable.is_some()
            || as_local_src.is_some()
            || as_local_kpar.is_some()
            || as_remote_src.is_some()
            || as_remote_kpar.is_some()
            || as_remote_git.is_some()
    }
}

#[derive(clap::Args, Debug, Clone)]
pub struct SourcesOptions {
    /// Do not include sources for dependencies
    #[arg(long, default_value = "deps")]
    pub deps: Dependencies,
    /// Do not include the project's own sources
    #[arg(long)]
    pub no_own: bool,
}

/// Selects which dependency sources a sources enumeration should yield. Whether
/// the project's own sources are listed is controlled separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Dependencies {
    /// No dependency sources.
    None,
    /// Dependency sources, excluding SysML v2/KerML standard libraries.
    Deps,
    /// Dependency sources, including SysML v2/KerML standard libraries.
    DepsStd,
    /// Only standard-library dependency sources.
    Std,
}

impl From<Dependencies> for CoreDependencies {
    fn from(val: Dependencies) -> Self {
        match val {
            Dependencies::None => Self::None,
            Dependencies::Deps => Self::Deps,
            Dependencies::DepsStd => Self::DepsStd,
            Dependencies::Std => Self::Std,
        }
    }
}

#[derive(clap::Args, Debug)]
pub struct GlobalOptions {
    /// Use verbose output
    #[arg(
        long,
        group = "log-level",
        global = true,
        help_heading = "Global options"
    )]
    pub verbose: bool,
    /// Do not output log messages
    #[arg(
        long,
        group = "log-level",
        global = true,
        help_heading = "Global options"
    )]
    pub quiet: bool,
    /// Disable discovery of configuration files
    #[arg(long, global = true, help_heading = "Global options", env = env_vars::SYSAND_NO_CONFIG)]
    pub no_config: bool,
    /// Give path to `sysand.toml` to use for configuration
    #[arg(long, global = true, help_heading = "Global options", env = env_vars::SYSAND_CONFIG_FILE)]
    pub config_file: Option<Utf8PathBuf>,
    /// Print help
    #[arg(long, global = true, action = clap::ArgAction::HelpLong, help_heading = "Global options")]
    pub help: Option<bool>,
}

/// Parse an IRI. Tolerates missing IRI scheme, uses
/// `https://` scheme in that case.
fn parse_https_iri(s: &str) -> Result<Iri<String>, fluent_uri::ParseError> {
    Iri::parse(s).map(Into::into).or_else(|original_err| {
        let scheme = "https://";
        let mut https = String::with_capacity(scheme.len() + s.len());
        https.push_str(scheme);
        https.push_str(s);
        // Return the original error to not confuse the user
        Iri::parse(https).map_err(|_irrelevant| original_err)
    })
}

/// Parse an SPDX license expression.
///
/// `spdx::ParseError` is a multiline diagram (the expression with a caret
/// under the offending term), so it must start on a line of its own to
/// stay aligned; clap puts the parser's error right after `...': `.
pub fn parse_spdx_expression(s: &str) -> Result<spdx::Expression, String> {
    spdx::Expression::parse(s).map_err(|err| {
        format!(
            "not a valid SPDX license expression:\n{err}\n\
            {USAGE}hint:{USAGE:#} {LICENSE_EXPRESSION_HELP}"
        )
    })
}

// Default metamodel for .kpar archives is KerML according to spec.
// But for non-packaged projects there is no default.
// Therefore, we don't provide a default here.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[clap(rename_all = "lowercase")]
pub enum MetamodelKind {
    /// SysML v2 metamodel. Identifier: `https://www.omg.org/spec/SysML/<release>`
    SysML,
    /// KerML metamodel. Identifier: `https://www.omg.org/spec/KerML/<release>`
    KerML,
}

impl From<&MetamodelKind> for &'static str {
    fn from(value: &MetamodelKind) -> Self {
        match value {
            MetamodelKind::SysML => SYSML_SPEC_PREFIX,
            MetamodelKind::KerML => KERML_SPEC_PREFIX,
        }
    }
}

impl From<MetamodelKind> for &'static str {
    fn from(value: MetamodelKind) -> Self {
        Self::from(&value)
    }
}

impl Display for MetamodelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metamodel(pub MetamodelKind, pub MetamodelVersion);

impl From<&Metamodel> for String {
    fn from(value: &Metamodel) -> Self {
        let mut s = Self::new();
        s.push_str(value.0.into());
        s.push_str(value.1.into());
        s
    }
}

impl Display for Metamodel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.into())?;
        f.write_str(self.1.into())
    }
}

impl From<Metamodel> for String {
    fn from(value: Metamodel) -> Self {
        Self::from(&value)
    }
}

#[expect(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetamodelVersion {
    Release_20250201 = 20250201,
}

impl From<&MetamodelVersion> for &'static str {
    fn from(value: &MetamodelVersion) -> Self {
        match value {
            MetamodelVersion::Release_20250201 => MetamodelVersion::RELEASE,
        }
    }
}

impl From<MetamodelVersion> for &'static str {
    fn from(value: MetamodelVersion) -> Self {
        Self::from(&value)
    }
}

impl MetamodelVersion {
    pub const RELEASE: &str = "20250201";
}

impl ValueEnum for MetamodelVersion {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Release_20250201]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(match self {
            Self::Release_20250201 => {
                PossibleValue::new(Self::RELEASE).help("SysMLv2/KerML Release or Beta4")
            }
        })
    }
}

/// Parse a `<publisher>/<name>` project identifier
pub fn parse_project_identifier(s: &str) -> Result<(ProjectPublisher, ProjectName), String> {
    let Some((publisher, name)) = s.split_once('/') else {
        return Err("identifier is not of the form `<publisher>/<name>`".to_owned());
    };
    Ok((
        parse_project_publisher(publisher).map_err(|e| e.to_string())?,
        parse_project_name(name).map_err(|e| e.to_string())?,
    ))
}

/// Parse a `<publisher>/<name>` identifier for an index project
pub fn parse_index_identifier(s: &str) -> Result<(IndexPublisher, IndexName), String> {
    let Some((publisher, name)) = s.split_once('/') else {
        return Err("identifier is not of the form `<publisher>/<name>`".to_owned());
    };
    Ok((
        IndexPublisher::parse(publisher.to_owned()).map_err(|(_, e)| e.to_string())?,
        IndexName::parse(name.to_owned()).map_err(|(_, e)| e.to_string())?,
    ))
}

/// Parse a project publisher
pub fn parse_project_publisher(s: &str) -> Result<ProjectPublisher, ProjectFieldError> {
    ProjectPublisher::parse(s.to_owned()).map_err(|(_, e)| e)
}

/// Parse a project name
pub fn parse_project_name(s: &str) -> Result<ProjectName, ProjectFieldError> {
    ProjectName::parse(s.to_owned()).map_err(|(_, e)| e)
}
