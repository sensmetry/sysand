// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    convert::Infallible,
    ffi::OsStr,
    fmt::{Display, Write},
};

use camino::Utf8PathBuf;
use clap::{ValueEnum, builder::StyledStr, crate_authors};
use semver::VersionReq;

use crate::env_vars;

/// A package manager for SysML v2 and KerML
///
/// Documentation:
/// <https://docs.sysand.org/>
/// Package index and more information:
/// <https://beta.sysand.org/>
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
        path: Option<String>,
        /// The name of the project. Defaults to the directory name
        #[arg(long)]
        name: Option<String>,
        /// Set the version in SemVer 2.0 format. Defaults to `0.0.1`
        #[arg(long)]
        version: Option<String>,
        /// Don't require version to conform to SemVer
        #[arg(long, requires = "version")]
        no_semver: bool,
        /// Set the license in the form of an SPDX license identifier
        /// Defaults to omitting the license field
        #[arg(long, alias = "licence", verbatim_doc_comment)]
        license: Option<String>,
        /// Don't require license to be an SPDX expression
        #[arg(long, requires = "license")]
        no_spdx: bool,
    },
    /// Add usage to project information
    Add {
        /// IRI identifying the project to be used
        iri: fluent_uri::Iri<String>,
        /// A constraint on the allowed versions of a used project.
        /// Assumes that the project being added uses Semantic Versioning.
        /// Version constraints use same syntax as Rust's Cargo.
        /// Examples: `1.2.3`, `<2`, `>=3`. For details, see the user
        /// guide's `Project information and metadata` section
        #[clap(verbatim_doc_comment)]
        version_constraint: Option<String>,
        /// Do not automatically resolve dependencies (and generate lockfile)
        #[arg(long, default_value = "false")]
        no_lock: bool,
        /// Do not automatically install dependencies
        #[arg(long, default_value = "false")]
        no_sync: bool,

        #[command(flatten)]
        resolution_opts: ResolutionOptions,
        #[command(flatten)]
        source_opts: ProjectSourceOptions,
    },
    /// Remove usage from project information
    #[clap(alias = "rm")]
    Remove {
        /// IRI identifying the project usage to be removed
        iri: fluent_uri::Iri<String>,
    },
    /// Clone a project to a specified directory.
    /// Equivalent to manually downloading, extracting the
    /// project to the directory and running `sysand sync`
    #[clap(verbatim_doc_comment)]
    Clone {
        #[clap(flatten)]
        locator: ProjectLocatorArgs,
        /// Path to clone the project into. If already exists, must
        /// be an empty directory. Defaults to current directory
        #[arg(long, short, default_value = None, verbatim_doc_comment)]
        target: Option<Utf8PathBuf>,
        /// Version of the project to clone. Defaults to the latest
        /// version according to SemVer 2.0
        #[arg(long, short = 'V', verbatim_doc_comment)]
        version: Option<String>,

        /// Don't resolve or install dependencies
        #[arg(long)]
        no_deps: bool,
        #[command(flatten)]
        resolution_opts: ResolutionOptions,
    },
    /// Include model interchange files in project metadata
    Include {
        /// File(s) to include in the project.
        #[arg(num_args = 1..)]
        paths: Vec<Utf8PathBuf>,
        /// Compute and add each file's (current) SHA256 checksum
        // TODO: will it ever be automatically updated?
        //       Maybe only when building a kpar?
        #[arg(long, default_value = "false")]
        compute_checksum: bool,
        /// Do not detect and add top level symbols to index
        #[arg(long, default_value = "false")]
        no_index_symbols: bool,
    },
    /// Exclude model interchange file from project metadata
    Exclude {
        /// File(s) to exclude from the project
        #[arg(num_args = 1..)]
        paths: Vec<String>,
    },
    /// Build a KerML Project Archive (KPAR). If executed in a workspace
    /// outside of a project, builds all projects in the workspace.
    #[clap(verbatim_doc_comment)]
    Build {
        /// Path giving where to put the finished KPAR or KPARs. When building a
        /// workspace, it is a path to the folder to write the KPARs to
        /// (default: `<current-workspace>/output`). When building a single
        /// project, it is a path to the KPAR file to write (default
        /// `<current-workspace>/output/<project name>-<version>.kpar` or
        /// `<current-project>/output/<project name>-<version>.kpar` depending
        /// on whether the current project belongs to a workspace or not).
        #[clap(verbatim_doc_comment)]
        path: Option<Utf8PathBuf>,
    },
    /// Create or update lockfile
    Lock {
        #[command(flatten)]
        resolution_opts: ResolutionOptions,
    },
    /// Create a local `sysand_env` directory for installing dependencies
    Env {
        #[command(subcommand)]
        command: Option<EnvCommand>,
    },
    /// Sync `sysand_env` to lockfile, creating a lockfile and `sysand_env` if needed
    Sync {
        #[command(flatten)]
        resolution_opts: ResolutionOptions,
    },
    /// Describe or modify a local project (either the current one
    /// or one at a given path) or resolve and describe a project
    /// at a specified path or IRI/URL
    #[clap(verbatim_doc_comment)]
    Info {
        /// Use the project at the given path instead of the current project
        #[arg(short = 'p', long, group = "location")]
        path: Option<String>,
        /// Use the project with the given IRI/URI/URL instead of the current project
        #[arg(
            short = 'i',
            long,
            visible_alias = "uri",
            visible_alias = "url",
            group = "location"
        )]
        iri: Option<fluent_uri::Iri<String>>,
        /// Use the project with the given locator, trying to parse it as
        /// an IRI/URI/URL and otherwise falling back to using it as a path
        #[arg(
            short = 'a',
            long,
            value_name = "LOCATOR",
            group = "location",
            verbatim_doc_comment
        )]
        auto_location: Option<String>,
        /// Do not try to normalise the IRI/URI when resolving
        #[arg(long, default_value = "false", visible_alias = "no-normalize")]
        no_normalise: bool,
        // TODO: Add various options, such as whether to take local environment
        //       into consideration
        #[command(flatten)]
        resolution_opts: ResolutionOptions,
        #[command(subcommand)]
        subcommand: Option<InfoCommand>,
    },
    /// List source files for the current project and
    /// (optionally) its dependencies
    Sources {
        #[command(flatten)]
        sources_opts: SourcesOptions,
    },
    /// Prints the root directory of the current project
    PrintRoot,
}

#[derive(clap::Args, Debug, Clone)]
#[group(required = true, multiple = false)]
pub struct ProjectLocatorArgs {
    /// Clone the project from a given locator, trying to parse it as an
    /// IRI/URI/URL and otherwise falling back to using it as a path
    #[clap(
        default_value = None,
        value_name = "LOCATOR", 
        verbatim_doc_comment
    )]
    pub auto_location: Option<String>,
    /// IRI/URI/URL identifying the project to be cloned
    #[arg(short = 'i', long, visible_alias = "uri", visible_alias = "url")]
    pub iri: Option<fluent_uri::Iri<String>>,
    /// Path to clone the project from. If version is also
    /// given, verifies that the project has the given version
    // TODO: allow somehow requiring to use git here
    #[arg(
        long,
        short = 's',
        default_value = None,
        value_name = "PATH",
        verbatim_doc_comment
    )]
    pub path: Option<String>,
}

#[derive(Clone, Debug)]
struct InvalidCommand {
    message: String,
}

fn invalid_command<S: AsRef<str>>(message: S) -> InvalidCommand {
    InvalidCommand {
        message: message.as_ref().to_string(),
    }
}

impl clap::builder::TypedValueParser for InvalidCommand {
    type Value = Infallible;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let mut err = clap::Error::new(clap::error::ErrorKind::UnknownArgument).with_cmd(cmd);
        if let Some(arg) = arg {
            err.insert(
                clap::error::ContextKind::InvalidArg,
                clap::error::ContextValue::String(arg.to_string()),
            );
        }
        err.insert(
            clap::error::ContextKind::InvalidValue,
            clap::error::ContextValue::String(value.to_string_lossy().to_string()),
        );

        // NOTE: https://github.com/clap-rs/clap/discussions/5318
        // Only works with StyledStrs
        let mut styled = StyledStr::new();
        styled.write_str(&self.message)?;
        err.insert(
            clap::error::ContextKind::Suggested,
            clap::error::ContextValue::StyledStrs(vec![styled]),
        );

        Err(err)
    }
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum InfoCommand {
    /// Get or set the name of the project
    #[group(required = false, multiple = false)]
    Name {
        #[arg(long, value_name = "NAME", default_value=None)]
        set: Option<String>,
        // Only for better error messages
        #[arg(hide = true, long, num_args=0, default_missing_value="None", value_parser=
            invalid_command("`name` cannot be unset"))]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=
            invalid_command("`name` is not a list, consider using `sysand info name --set`?"))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=
            invalid_command("`name` is not a list, and cannot be unset"))]
        remove: Option<Infallible>,
    },
    /// Get or set the description of the project
    #[group(required = false, multiple = false)]
    Description {
        #[arg(long, value_name = "DESCRIPTION", default_value=None)]
        set: Option<String>,
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`description` is not a list, consider using `sysand info description --set`?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`description` is not a list, consider using `sysand info description --clear`?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or set the version of the project
    #[group(required = false, multiple = false)]
    Version {
        /// Set the version in SemVer 2.0 format
        #[arg(long, value_name = "VERSION", default_value=None)]
        set: Option<String>,
        /// Don't require version to conform to Semantic Versioning
        #[arg(long, requires = "set")]
        no_semver: bool,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            default_value = None,
            value_parser=invalid_command("`version` cannot be unset")
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`version` is not a list, consider using `sysand info version --set`?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`version` is not a list, and cannot be unset"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or set the license of the project
    #[command(visible_alias = "licence")]
    #[group(required = false, multiple = false)]
    License {
        /// Set the license in the form of an SPDX license identifier
        #[arg(long, value_name = "LICENSE", default_value=None)]
        set: Option<String>,
        /// Don't require license to be an SPDX expression
        #[arg(long, requires = "set")]
        no_spdx: bool,
        /// Remove the project's license
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`license` is not a list, consider using `sysand info license --set`?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`license` is not a list, consider using `sysand info license --clear`?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or manipulate the list of maintainers of the project
    #[group(required = false, multiple = false)]
    Maintainer {
        #[arg(long, value_name = "MAINTAINER", default_value=None)]
        set: Option<String>,
        #[arg(long, default_value = None)]
        clear: bool,
        #[arg(long, default_value=None)]
        add: Option<String>,
        #[arg(long, default_value=None)]
        remove: Option<usize>,
        /// Prints a numbered list
        #[arg(long, default_value = "false")]
        numbered: bool,
    },
    /// Get or set the website of the project
    #[group(required = false, multiple = false)]
    Website {
        /// Set the website. Must be a valid IRI/URI/URL
        #[arg(long, value_name = "URI", value_parser = parse_https_iri, default_value=None)]
        set: Option<fluent_uri::Iri<String>>,
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`website` is not a list, consider using `sysand info website --set`?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`website` is not a list, consider using `sysand info website --clear`?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or manipulate the list of topics of the project
    #[group(required = false, multiple = false)]
    Topic {
        #[arg(long, value_name = "TOPIC", default_value=None)]
        set: Option<String>,
        #[arg(long, default_value = None)]
        clear: bool,
        #[arg(long, value_name = "TOPIC", default_value=None)]
        add: Option<String>,
        #[arg(long, default_value=None)]
        remove: Option<usize>,
        /// Prints a numbered list
        #[arg(long, default_value = "false")]
        numbered: bool,
    },
    /// Print project usages
    #[group(required = false, multiple = false)]
    Usage {
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`usage` cannot be set directly, please use `sysand add` and `sysand remove`"
        ))]
        set: Option<Infallible>,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            value_parser=invalid_command(
              "`usage` cannot be cleared directly, please use `sysand remove`"
            )
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`usage` cannot be added to directly, please use `sysand add`"
        ))]
        add: Option<Infallible>,
        // Only for Infallible error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`usage` cannot be removed from directly, please use `sysand remove`"
        ))]
        remove: Option<Infallible>,
        /// Prints a numbered list
        #[arg(long, default_value = "false")]
        numbered: bool,
    },
    /// Get project index
    #[group(required = false, multiple = false)]
    Index {
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`index` cannot be set directly, please use `sysand include` and `sysand exclude`"
        ))]
        set: Option<Infallible>,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            value_parser=invalid_command(
              "`index` cannot be cleared directly, please use `sysand exclude`"
            )
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`index` cannot be added to directly, please use `sysand include` and `sysand exclude`"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`index` cannot be removed from directly, please use `sysand exclude`"
        ))]
        remove: Option<Infallible>,
        /// Prints a numbered list
        #[arg(long, default_value = "false")]
        numbered: bool,
    },
    /// Get project metadata manifest creation time
    #[group(required = false, multiple = false)]
    Created {
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`created` cannot be set directly, it is automatically updated"
        ))]
        set: Option<Infallible>,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            value_parser=invalid_command(
              "`created` cannot be cleared, it is automatically updated"
            )
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`created` cannot be added to, it is automatically updated"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`created` cannot be removed from, it is automatically updated"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or set the metamodel of the project
    #[group(required = false)]
    Metamodel {
        // It would be nicer to have Option<Metamodel> here,
        // but that would introduce an additional level of
        // nesting, as clap does not support flatten with Option
        /// Set a SysML v2 or KerML metamodel. To set a custom metamodel, use `--set-custom`
        #[arg(long, value_name = "KIND", value_enum, default_value=None)]
        set: Option<MetamodelKind>,
        /// Choose the release of the SysML v2 or KerML metamodel.
        /// SysML 2.0 and KerML 1.0 have the same release dates
        #[arg(
            long,
            value_name = "YYYYMMDD",
            requires = "set",
            value_enum,
            verbatim_doc_comment,
            default_value=MetamodelVersion::RELEASE
        )]
        release: MetamodelVersion,
        /// Choose a custom release of the SysML v2 or KerML metamodel.
        #[arg(
            long,
            value_name = "YYYYMMDD",
            requires = "set",
            conflicts_with = "release",
            default_value=None,
        )]
        release_custom: Option<u32>,
        /// Set a custom metamodel. To set a SysML v2 or KerML metamodel, use `--set`
        #[arg(
            long,
            value_name = "METAMODEL",
            conflicts_with_all = ["set", "release", "release_custom"],
            default_value=None
        )]
        set_custom: Option<String>,
        #[arg(long, num_args=0, default_missing_value="true", default_value = None, conflicts_with = "set")]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`metamodel` is not a list, consider using `sysand info metamodel --set`?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`metamodel` is not a list, consider using `sysand info metamodel --clear`?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or set whether the project includes derived properties
    #[group(required = false, multiple = false)]
    IncludesDerived {
        #[arg(long, value_name = "INCLUDES_DERIVED", num_args=1, default_value=None)]
        set: Option<bool>,
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(
            hide=true,
            long,
            default_value=None,
            value_parser=invalid_command(
            "`include_derived` is not a list, consider using `sysand info include_derived --set`?"
            )
        )]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true,
          long,
          default_value=None,
          value_parser=invalid_command(
          "`include_derived` is not a list, consider using `sysand info include_derived --clear`?"
          )
        )]
        remove: Option<Infallible>,
    },
    /// Get or set whether the project includes implied properties
    #[group(required = false, multiple = false)]
    IncludesImplied {
        #[arg(long, value_name = "INCLUDES_IMPLIED", num_args=1, default_value=None)]
        set: Option<bool>,
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`include_implied` is not a list, consider using `sysand info include_implied --set`?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`include_implied` is not a list, consider using `sysand info include_implied --clear`?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get project source file checksums
    #[group(required = false, multiple = false)]
    Checksum {
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`checksum` cannot be set directly, please use `sysand include` and `sysand exclude`"
        ))]
        set: Option<Infallible>,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            value_parser=invalid_command(
              "`checksum` cannot be cleared directly, please use `sysand exclude`"
            )
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`checksum` cannot be added to directly, please use `sysand include`"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "`checksum` cannot be removed from directly, please use `sysand exclude`"
        ))]
        remove: Option<Infallible>,
        /// Prints a numbered list
        #[arg(long, default_value = "false")]
        numbered: bool,
    },
}

#[derive(Debug, Clone)]
pub enum InfoCommandVerb {
    Get(GetVerb),
    Set(SetVerb),
    Clear(ClearVerb),
    Add(AddVerb),
    Remove(RemoveVerb),
}

#[derive(Debug, Clone)]
pub enum GetVerb {
    GetInfoVerb(GetInfoVerb),
    GetMetaVerb(GetMetaVerb),
}

#[derive(Debug, Clone)]
pub enum SetVerb {
    SetInfoVerb(SetInfoVerb),
    SetMetaVerb(SetMetaVerb),
}

#[derive(Debug, Clone)]
pub enum ClearVerb {
    ClearInfoVerb(ClearInfoVerb),
    ClearMetaVerb(ClearMetaVerb),
}

#[derive(Debug, Clone)]
pub enum AddVerb {
    AddInfoVerb(AddInfoVerb),
    AddMetaVerb(AddMetaVerb),
}

#[derive(Debug, Clone)]
pub enum RemoveVerb {
    RemoveInfoVerb(RemoveInfoVerb),
    RemoveMetaVerb(RemoveMetaVerb),
}

#[derive(Debug, Clone)]
pub enum GetInfoVerb {
    GetName,
    GetDescription,
    GetVersion,
    GetLicense,
    GetMaintainer,
    GetWebsite,
    GetTopic,
    GetUsage,
}

#[derive(Debug, Clone)]
pub enum SetInfoVerb {
    SetName(String),
    SetDescription(String),
    SetVersion(String),
    SetLicense(String),
    SetMaintainer(Vec<String>),
    SetWebsite(String),
    SetTopic(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum ClearInfoVerb {
    ClearDescription,
    ClearLicense,
    ClearMaintainer,
    ClearWebsite,
    ClearTopic,
}

#[derive(Debug, Clone)]
pub enum AddInfoVerb {
    AddMaintainer(Vec<String>),
    AddTopic(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum RemoveInfoVerb {
    RemoveMaintainer(usize),
    RemoveTopic(usize),
}

#[derive(Debug, Clone)]
pub enum GetMetaVerb {
    GetIndex,
    GetCreated,
    GetMetamodel,
    GetIncludesDerived,
    GetIncludesImplied,
    GetChecksum,
}

#[derive(Debug, Clone)]
pub enum SetMetaVerb {
    SetMetamodel(String),
    SetIncludesDerived(bool),
    SetIncludesImplied(bool),
}

#[derive(Debug, Clone)]
pub enum ClearMetaVerb {
    ClearMetamodel,
    ClearIncludesDerived,
    ClearIncludesImplied,
}

#[derive(Debug, Clone)]
pub enum AddMetaVerb {
    // Currently nothing
}

#[derive(Debug, Clone)]
pub enum RemoveMetaVerb {
    // Currently nothing
}

impl InfoCommand {
    pub fn as_verb(self) -> InfoCommandVerb {
        fn pack(
            get: GetVerb,
            set: Option<SetVerb>,
            clear: Option<ClearVerb>,
            add: Option<AddVerb>,
            remove: Option<RemoveVerb>,
        ) -> InfoCommandVerb {
            match (set, clear, add, remove) {
                (None, None, None, None) => InfoCommandVerb::Get(get),
                (Some(set), None, None, None) => InfoCommandVerb::Set(set),
                (None, Some(clear), None, None) => InfoCommandVerb::Clear(clear),
                (None, None, Some(add), None) => InfoCommandVerb::Add(add),
                (None, None, None, Some(remove)) => InfoCommandVerb::Remove(remove),
                _ => unreachable!("internal error: invalid CLI command produced"),
            }
        }

        fn pack_info(
            get: GetInfoVerb,
            set: Option<SetInfoVerb>,
            clear: Option<ClearInfoVerb>,
            add: Option<AddInfoVerb>,
            remove: Option<RemoveInfoVerb>,
        ) -> InfoCommandVerb {
            pack(
                GetVerb::GetInfoVerb(get),
                set.map(SetVerb::SetInfoVerb),
                clear.map(ClearVerb::ClearInfoVerb),
                add.map(AddVerb::AddInfoVerb),
                remove.map(RemoveVerb::RemoveInfoVerb),
            )
        }

        fn pack_meta(
            get: GetMetaVerb,
            set: Option<SetMetaVerb>,
            clear: Option<ClearMetaVerb>,
            add: Option<AddMetaVerb>,
            remove: Option<RemoveMetaVerb>,
        ) -> InfoCommandVerb {
            pack(
                GetVerb::GetMetaVerb(get),
                set.map(SetVerb::SetMetaVerb),
                clear.map(ClearVerb::ClearMetaVerb),
                add.map(AddVerb::AddMetaVerb),
                remove.map(RemoveVerb::RemoveMetaVerb),
            )
        }

        fn impossible<T>(impossible: Option<Infallible>) -> Option<T> {
            impossible.map(|x| match x {})
        }

        match self {
            InfoCommand::Name {
                set,
                clear,
                add,
                remove,
            } => pack_info(
                GetInfoVerb::GetName,
                set.map(SetInfoVerb::SetName),
                impossible(clear),
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::Description {
                set,
                clear,
                add,
                remove,
            } => pack_info(
                GetInfoVerb::GetDescription,
                set.map(SetInfoVerb::SetDescription),
                if clear {
                    Some(ClearInfoVerb::ClearDescription)
                } else {
                    None
                },
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::Version {
                set,
                clear,
                add,
                remove,
                no_semver: _,
            } => pack_info(
                GetInfoVerb::GetVersion,
                set.map(SetInfoVerb::SetVersion),
                impossible(clear),
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::License {
                set,
                clear,
                add,
                remove,
                no_spdx: _,
            } => pack_info(
                GetInfoVerb::GetLicense,
                set.map(SetInfoVerb::SetLicense),
                if clear {
                    Some(ClearInfoVerb::ClearLicense)
                } else {
                    None
                },
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::Maintainer {
                set,
                clear,
                add,
                remove,
                numbered: _,
            } => pack_info(
                GetInfoVerb::GetMaintainer,
                set.map(|x| SetInfoVerb::SetMaintainer(vec![x])),
                if clear {
                    Some(ClearInfoVerb::ClearMaintainer)
                } else {
                    None
                },
                add.map(|x| AddInfoVerb::AddMaintainer(vec![x])),
                remove.map(RemoveInfoVerb::RemoveMaintainer),
            ),
            InfoCommand::Website {
                set,
                clear,
                add,
                remove,
            } => pack_info(
                GetInfoVerb::GetWebsite,
                set.map(|i| SetInfoVerb::SetWebsite(i.into_string())),
                if clear {
                    Some(ClearInfoVerb::ClearWebsite)
                } else {
                    None
                },
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::Topic {
                set,
                clear,
                add,
                remove,
                numbered: _,
            } => pack_info(
                GetInfoVerb::GetTopic,
                set.map(|x| SetInfoVerb::SetTopic(vec![x])),
                if clear {
                    Some(ClearInfoVerb::ClearTopic)
                } else {
                    None
                },
                add.map(|x| AddInfoVerb::AddTopic(vec![x])),
                remove.map(RemoveInfoVerb::RemoveTopic),
            ),
            InfoCommand::Usage {
                set,
                clear,
                add,
                remove,
                numbered: _,
            } => pack_info(
                GetInfoVerb::GetUsage,
                impossible(set),
                impossible(clear),
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::Index {
                set,
                clear,
                add,
                remove,
                numbered: _,
            } => pack_meta(
                GetMetaVerb::GetIndex,
                impossible(set),
                impossible(clear),
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::Created {
                set,
                clear,
                add,
                remove,
            } => pack_meta(
                GetMetaVerb::GetCreated,
                impossible(set),
                impossible(clear),
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::Metamodel {
                set,
                release,
                release_custom,
                set_custom,
                clear,
                add,
                remove,
            } => {
                let metamodel = match set {
                    Some(mk) => match release_custom {
                        Some(rc) => Some(format!("{mk}{rc}")),
                        None => Some(Metamodel(mk, release).into()),
                    },
                    None => set_custom,
                };
                pack_meta(
                    GetMetaVerb::GetMetamodel,
                    metamodel.map(SetMetaVerb::SetMetamodel),
                    if clear {
                        Some(ClearMetaVerb::ClearMetamodel)
                    } else {
                        None
                    },
                    impossible(add),
                    impossible(remove),
                )
            }
            InfoCommand::IncludesDerived {
                set,
                clear,
                add,
                remove,
            } => pack_meta(
                GetMetaVerb::GetIncludesDerived,
                set.map(SetMetaVerb::SetIncludesDerived),
                if clear {
                    Some(ClearMetaVerb::ClearIncludesDerived)
                } else {
                    None
                },
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::IncludesImplied {
                set,
                clear,
                add,
                remove,
            } => pack_meta(
                GetMetaVerb::GetIncludesImplied,
                set.map(SetMetaVerb::SetIncludesImplied),
                if clear {
                    Some(ClearMetaVerb::ClearIncludesImplied)
                } else {
                    None
                },
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::Checksum {
                set,
                clear,
                add,
                remove,
                numbered: _,
            } => pack_meta(
                GetMetaVerb::GetChecksum,
                impossible(set),
                impossible(clear),
                impossible(add),
                impossible(remove),
            ),
        }
    }

    pub fn numbered(&self) -> bool {
        // NOTE: Avoid using { .. } here, in order to not accidentally miss the introduction of
        //       relevant flags in the future.
        match self {
            InfoCommand::Name {
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::Description {
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::Version {
                set: _,
                no_semver: _,
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::License {
                set: _,
                no_spdx: _,
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::Maintainer {
                numbered,
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => *numbered,
            InfoCommand::Website {
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::Topic {
                numbered,
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => *numbered,
            InfoCommand::Usage {
                numbered,
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => *numbered,
            InfoCommand::Index {
                numbered,
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => *numbered,
            InfoCommand::Created {
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::Metamodel {
                set: _,
                release: _,
                release_custom: _,
                set_custom: _,
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::IncludesDerived {
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::IncludesImplied {
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::Checksum {
                numbered,
                set: _,
                clear: _,
                add: _,
                remove: _,
            } => *numbered,
        }
    }
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum EnvCommand {
    /// Install project in `sysand_env`
    Install {
        /// IRI identifying the project to be installed
        iri: fluent_uri::Iri<String>,
        /// Version to be installed. Defaults to the latest
        /// version according to SemVer 2.0, ignoring pre-releases
        #[clap(verbatim_doc_comment)]
        version: Option<String>,
        /// Path to interchange project
        #[arg(long, default_value = None)]
        path: Option<Utf8PathBuf>,

        #[command(flatten)]
        install_opts: InstallOptions,
        #[command(flatten)]
        resolution_opts: ResolutionOptions,
    },
    /// Uninstall project in `sysand_env`
    Uninstall {
        /// IRI identifying the project to be uninstalled
        iri: fluent_uri::Iri<String>,
        /// Version to be uninstalled
        version: Option<String>,
    },
    /// List projects installed in `sysand_env`
    List,
    /// List source files for an installed project and
    /// (optionally) its dependencies
    #[clap(verbatim_doc_comment)]
    Sources {
        /// IRI of the (already installed) project for which
        /// to enumerate source files
        #[clap(verbatim_doc_comment)]
        iri: fluent_uri::Iri<String>,
        /// Version of project to list sources for
        version: Option<VersionReq>,

        #[command(flatten)]
        sources_opts: SourcesOptions,
    },
}

#[derive(clap::Args, Debug, Clone)]
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
pub struct ResolutionOptions {
    /// Comma-delimited list of index URLs to use when resolving
    /// project(s) and/or their dependencies, in addition to the default indexes.
    #[arg(
        long,
        num_args = 0..,
        global = true,
        help_heading = "Resolution options",
        env = env_vars::SYSAND_INDEX,
        value_delimiter = ',',
        verbatim_doc_comment
    )]
    pub index: Vec<String>,
    /// Comma-delimited list of URLs to use as default index
    /// URLs. Default indexes are tried before other indexes
    /// (default `https://beta.sysand.org`)
    // TODO: verify index use order
    #[arg(
        long,
        num_args = 0..,
        global = true,
        help_heading = "Resolution options",
        env = env_vars::SYSAND_DEFAULT_INDEX,
        value_delimiter = ',',
        verbatim_doc_comment
    )]
    pub default_index: Vec<String>,
    /// Do not use any index when resolving project(s) and/or their dependencies
    // TODO: document somewhere which sources are supported:
    // - file:// (sometimes also regular paths)
    // - git (https?, ssh?)
    // - http(s)
    // - index
    #[arg(
        long,
        default_value = "false",
        conflicts_with_all = ["index", "default_index"],
        global = true,
        help_heading = "Resolution options",
    )]
    pub no_index: bool,
    /// Don't ignore KerML/SysML v2 standard libraries if specified as dependencies
    #[arg(
        long,
        default_value = "false",
        global = true,
        help_heading = "Resolution options"
    )]
    pub include_std: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct ProjectSourceOptions {
    /// Add usage as a local interchange project at PATH and
    /// update configuration file attempting to guess the
    /// source from the PATH
    #[arg(long, value_name = "PATH", group = "source")]
    pub from_path: Option<String>,
    /// Add usage as a remote interchange project at URL and
    /// update configuration file attempting to guess the
    /// source from the URL
    #[arg(long, value_name = "URL", group = "source")]
    pub from_url: Option<String>,
    /// Add usage as an editable interchange project at PATH and
    /// update configuration file with appropriate source
    #[arg(long, value_name = "PATH", group = "source")]
    pub as_editable: Option<String>,
    /// Add usage as a local interchange project at PATH and
    /// update configuration file with appropriate source
    #[arg(long, value_name = "PATH", group = "source")]
    pub as_local_src: Option<String>,
    /// Add usage as a local interchange project archive at PATH
    /// and update configuration file with appropriate source
    #[arg(long, value_name = "PATH", group = "source")]
    pub as_local_kpar: Option<String>,
    /// Add usage as a remote interchange project at URL and
    /// update configuration file with appropriate source
    #[arg(long, value_name = "URL", group = "source")]
    pub as_remote_src: Option<String>,
    /// Add usage as a remote interchange project archive at URL
    /// and update configuration file with appropriate source
    #[arg(long, value_name = "URL", group = "source")]
    pub as_remote_kpar: Option<String>,
    /// Add usage as a remote git interchange project at URL and
    /// update configuration file with appropriate source
    #[arg(long, value_name = "URL", group = "source")]
    pub as_remote_git: Option<String>,
}

#[derive(clap::Args, Debug, Clone)]
pub struct SourcesOptions {
    /// Do not include sources for dependencies
    #[arg(long, default_value = "false", conflicts_with = "include_std")]
    pub no_deps: bool,
    /// Include (installed) KerML/SysML v2 standard libraries
    #[arg(long, default_value = "false")]
    pub include_std: bool,
}

#[derive(clap::Args, Debug)]
pub struct GlobalOptions {
    /// Use verbose output
    #[arg(
        long,
        short,
        group = "log-level",
        global = true,
        help_heading = "Global options"
    )]
    pub verbose: bool,
    /// Do not output log messages
    #[arg(
        long,
        short,
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
    pub config_file: Option<String>,
    /// Print help
    #[arg(long, short, global = true, action = clap::ArgAction::HelpLong, help_heading = "Global options")]
    pub help: Option<bool>,
}

/// Parse an IRI. Tolerates missing IRI scheme, uses
/// `https://` scheme in that case.
fn parse_https_iri(s: &str) -> Result<fluent_uri::Iri<String>, fluent_uri::ParseError> {
    use fluent_uri::Iri;

    Iri::parse(s).map(Into::into).or_else(|original_err| {
        let scheme = "https://";
        let mut https = String::with_capacity(scheme.len() + s.len());
        https.push_str(scheme);
        https.push_str(s);
        // Return the original error to not confuse the user
        Iri::parse(https).map_err(|_| original_err)
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

impl MetamodelKind {
    pub const SYSML: &str = "https://www.omg.org/spec/SysML/";
    pub const KERML: &str = "https://www.omg.org/spec/KerML/";
}

impl From<&MetamodelKind> for &'static str {
    fn from(value: &MetamodelKind) -> Self {
        match value {
            MetamodelKind::SysML => MetamodelKind::SYSML,
            MetamodelKind::KerML => MetamodelKind::KERML,
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
        let mut s = String::new();
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

#[allow(non_camel_case_types)]
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
        use clap::builder::PossibleValue;
        Some(match self {
            MetamodelVersion::Release_20250201 => {
                PossibleValue::new(MetamodelVersion::RELEASE).help("SysMLv2/KerML Release or Beta4")
            }
        })
    }
}
