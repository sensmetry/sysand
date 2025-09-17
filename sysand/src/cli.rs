// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use semver::VersionReq;

const DEFAULT_INDEX_URL: &str = "https://beta.sysand.org";

/// A project manager for KerML and SysML
#[derive(clap::Parser, Debug)]
#[command(author, version, about, long_about = None, arg_required_else_help = true)]
#[command(styles=crate::style::STYLING)]
pub struct Args {
    #[command(flatten)]
    pub global_opts: GlobalOpts,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum Command {
    /// Create new project in current directory
    Init {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
    /// Create new project in given directory
    New {
        dir: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
    /// Create a local sysand_env environment for installing dependencies
    Env {
        #[command(subcommand)]
        command: Option<EnvCommand>,
    },
    /// Sync env to lockfile
    Sync,
    /// Prints the root directory of the current project
    PrintRoot,
    /// Resolve and describe current interchange project or one at at a specified path or IRI/URL.
    Info {
        /// Use local path instead of IRI/URI (set by default).
        #[arg(short = 'p', long, group = "location-kind", requires = "location")]
        path: bool,
        /// Use IRI/URI instead of local path.
        #[arg(
            short = 'i',
            long,
            visible_alias = "uri",
            group = "location-kind",
            requires = "location"
        )]
        iri: bool,
        /// Automatically detect the location kind by first trying to parse it
        /// as an IRI/URI and then falling back to a local path.
        #[arg(short = 'a', long, group = "location-kind", requires = "location")]
        auto: bool,
        /// Local path or IRI/URI of interchange project
        #[arg(default_value = None)]
        location: Option<String>,
        /// Do not try to normalise the IRI/URI when resolving
        #[arg(long, default_value = "false", visible_alias = "no-normalize")]
        no_normalise: bool,
        /// Use an index when resolving this usage
        #[arg(long, default_value = Some(DEFAULT_INDEX_URL))]
        use_index: Option<String>,
        /// Do not use any index when resolving this usage
        #[arg(long, default_value = "false", conflicts_with = "use_index")]
        no_index: bool,
        // TODO: Add various options, such as whether to take local environment
        //       into consideration
    },
    /// Update lockfile
    Lock {
        /// Use an index when updating the lockfile
        #[arg(long, default_value = Some(DEFAULT_INDEX_URL))]
        use_index: Option<String>,
        /// Do not use any index when updating the lockfile
        #[arg(long, default_value = "false", conflicts_with = "use_index")]
        no_index: bool,
    },
    /// Add usage to project information
    Add {
        /// IRI identifying the project to be used.
        iri: String,
        /// A constraint on the allowable versions of a used project.
        versions_constraint: Option<String>,
        /// Do not automatically resolve usages (and generate lockfile)
        #[arg(long, default_value = "false")]
        no_lock: bool,
        /// Do not automatically install dependencies
        #[arg(long, default_value = "false")]
        no_sync: bool,
        /// Use an index when resolving this usage
        #[arg(long, default_value = Some(DEFAULT_INDEX_URL))]
        use_index: Option<String>,
        /// Do not use any index when resolving this usage
        #[arg(long, default_value = "false", conflicts_with = "use_index")]
        no_index: bool,
    },
    /// Remove usage from project information
    Remove {
        /// IRI identifying the project used.
        iri: String,
    },
    /// Include model interchange files in project metadata
    Include {
        /// File to include in the project.
        #[arg(num_args = 1..)]
        paths: Vec<String>,
        /// Compute and add file (current) SHA256 checksum.
        #[arg(long, default_value = "false")]
        compute_checksum: bool,
        /// Do not detect and add top level symbols to index.
        #[arg(long, default_value = "false")]
        no_index_symbols: bool,
    },
    /// Exclude model interchange file from project metadata
    Exclude {
        /// Files to exclude from the project.
        #[arg(num_args = 1..)]
        paths: Vec<String>,
    },
    /// Build kpar
    Build {
        /// Path giving where to put the finished kpar
        path: Option<std::path::PathBuf>,
    },
    /// Enumerate source files for the current project and
    /// (optionally) its dependencies.
    Sources {
        /// Do not include the project dependencies
        #[arg(long, default_value = "false")]
        no_deps: bool,
    },
}

#[derive(clap::Subcommand, Debug, Clone)]
pub enum EnvCommand {
    /// Install project in sysand_env
    Install {
        iri: String,
        version: Option<String>,
        /// Local path to interchange project
        #[arg(long, default_value = None)]
        location: Option<String>,
        /// Local path to index
        #[arg(long, default_value = None)]
        index: Option<String>,
        /// Allow overwriting existing installation
        #[arg(long)]
        allow_overwrite: bool,
        /// Install even if another version is already installed
        #[arg(long)]
        allow_multiple: bool,
    },
    /// Uninstall project in sysand_env
    Uninstall {
        iri: String,
        version: Option<String>,
    },
    /// List projects installed in sysand_env
    List,
    /// Enumerate source files for an installed project and
    /// (optionally) its dependencies.
    Sources {
        /// IRI of the (already installed) project for which
        /// to enumerate source files
        iri: String,
        /// Version of project to list sources for
        #[arg(long, default_value = None)]
        version: Option<VersionReq>,
        /// Do not include the project dependencies
        #[arg(long, default_value = "false")]
        no_deps: bool,
    },
}

// #[derive(clap::Args, Debug)]
// pub struct EnvOptions {}

#[derive(clap::Args, Debug)]
pub struct GlobalOpts {
    /// Use verbose output
    #[arg(long, short, group = "log-level", global = true)]
    pub verbose: bool,
    /// Do not output log messages
    #[arg(long, short, group = "log-level", global = true)]
    pub quiet: bool,
    /// Disable discovery of configuration files
    #[arg(long, short, global = true)]
    pub no_config: bool,
    /// Give path to 'sysand.toml' to use for configuration
    #[arg(long, short, global = true)]
    pub config_file: Option<String>,
}

impl GlobalOpts {
    pub fn sets_log_level(&self) -> bool {
        self.verbose || self.quiet
    }
}
