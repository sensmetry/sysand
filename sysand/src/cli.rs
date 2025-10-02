// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{convert::Infallible, fmt::Write};

use clap::builder::StyledStr;
use semver::VersionReq;

const DEFAULT_INDEX_URL: &str = "https://beta.sysand.org";

/// A project manager for KerML and SysML
#[derive(clap::Parser, Debug)]
#[command(author, version, about, long_about = None, arg_required_else_help = true)]
#[command(styles=crate::style::STYLING)]
pub struct Args {
    #[command(flatten)]
    pub global_opts: GlobalOptions,

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
    /// Sync env to lockfile, creating a lockfile if none is found
    Sync {
        #[command(flatten)]
        dependency_opts: DependencyOptions,
    },
    /// Prints the root directory of the current project
    PrintRoot,
    /// Resolve and describe current interchange project or one at at a specified path or IRI/URL.
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
        iri: Option<String>,
        /// Use the project with the given location, trying to parse it
        /// as an IRI/URI/URL and otherwise falling back to a local path
        #[arg(short = 'a', long, group = "location")]
        auto_location: Option<String>,
        /// Do not try to normalise the IRI/URI when resolving
        #[arg(long, default_value = "false", visible_alias = "no-normalize")]
        no_normalise: bool,
        // TODO: Add various options, such as whether to take local environment
        //       into consideration
        #[command(flatten)]
        dependency_opts: DependencyOptions,
        #[command(subcommand)]
        subcommand: Option<InfoCommand>,
    },
    /// Update lockfile
    Lock {
        #[command(flatten)]
        dependency_opts: DependencyOptions,
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

        #[command(flatten)]
        dependency_opts: DependencyOptions,
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
        #[command(flatten)]
        sources_opts: SourcesOptions,
    },
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
        value: &std::ffi::OsStr,
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
        #[arg(long, default_value=None)]
        set: Option<String>,
        // Only for better error messages
        #[arg(hide = true, long, num_args=0, default_missing_value="None", value_parser=
            invalid_command("'name' cannot be unset"))]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=
            invalid_command("'name' is not a list, consider using 'sysand info name --set'?"))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=
            invalid_command("'name' is not a list, and cannot be unset"))]
        remove: Option<Infallible>,
    },
    /// Get or set the description of the project
    #[group(required = false, multiple = false)]
    Description {
        #[arg(long, default_value=None)]
        set: Option<String>,
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'description' is not a list, consider using 'sysand info description --set'?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'description' is not a list, consider using 'sysand info description --clear'?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or set the version of the project
    #[group(required = false, multiple = false)]
    Version {
        #[arg(long, default_value=None)]
        set: Option<String>,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            default_value = None,
            value_parser=invalid_command("'version' cannot be unset")
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'version' is not a list, consider using 'sysand info version --set'?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'version' is not a list, and cannot be unset"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or set the licence of the project
    #[command(visible_alias = "license")]
    #[group(required = false, multiple = false)]
    Licence {
        #[arg(long, default_value=None)]
        set: Option<String>,
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'licence' is not a list, consider using 'sysand info licence --set'?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'licence' is not a list, consider using 'sysand info licence --clear'?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or manipulate the list of maintainers of the project
    #[group(required = false, multiple = false)]
    Maintainer {
        #[arg(long, default_value=None)]
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
        #[arg(long, default_value=None)]
        set: Option<String>,
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'website' is not a list, consider using 'sysand info website --set'?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'website' is not a list, consider using 'sysand info website --clear'?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or manipulate the list of topics of the project
    #[group(required = false, multiple = false)]
    Topic {
        #[arg(long, default_value=None)]
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
    /// Print project usages
    #[group(required = false, multiple = false)]
    Usage {
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'usage' cannot be set directly, please use 'sysand add' and 'sysand remove'"
        ))]
        set: Option<Infallible>,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            value_parser=invalid_command(
              "'usage' cannot be cleared directly, please use 'sysand remove'"
            )
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'usage' cannot be added to directly, please use 'sysand add'"
        ))]
        add: Option<Infallible>,
        // Only for Infallible error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'usage' cannot be removed from directly, please use 'sysand remove'"
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
          "'index' cannot be set directly, please use 'sysand include' and 'sysand exclude'"
        ))]
        set: Option<Infallible>,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            value_parser=invalid_command(
              "'index' cannot be cleared directly, please use 'sysand exclude'"
            )
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'index' cannot be added to directly, please use 'sysand include' and 'sysand exclude'"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'index' cannot be removed from directly, please use 'sysand exclude'"
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
          "'created' cannot be set directly, it is automatically updated"
        ))]
        set: Option<Infallible>,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            value_parser=invalid_command(
              "'created' cannot be cleared, it is automatically updated"
            )
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'created' cannot be added to, it is automatically updated"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'created' cannot be removed from, it is automatically updated"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or set the metamodel of the project
    #[group(required = false, multiple = false)]
    Metamodel {
        #[arg(long, default_value=None)]
        set: Option<String>,
        #[arg(long, num_args=0, default_missing_value="true", default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'metamodel' is not a list, consider using 'sysand info metamodel --set'?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'metamodel' is not a list, consider using 'sysand info metamodel --clear'?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get or set whether the project includes derived properties
    #[group(required = false, multiple = false)]
    IncludesDerived {
        #[arg(long, num_args=1, default_value=None)]
        set: Option<bool>,
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(
            hide=true,
            long,
            default_value=None,
            value_parser=invalid_command(
            "'include_derived' is not a list, consider using 'sysand info include_derived --set'?"
            )
        )]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true,
          long,
          default_value=None,
          value_parser=invalid_command(
          "'include_derived' is not a list, consider using 'sysand info include_derived --clear'?"
          )
        )]
        remove: Option<Infallible>,
    },
    /// Get or set whether the project includes implied properties
    #[group(required = false, multiple = false)]
    IncludesImplied {
        #[arg(long, num_args=1, default_value=None)]
        set: Option<bool>,
        #[arg(long, default_value = None)]
        clear: bool,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'include_implied' is not a list, consider using 'sysand info include_implied --set'?"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "'include_implied' is not a list, consider using 'sysand info include_implied --clear'?"
        ))]
        remove: Option<Infallible>,
    },
    /// Get project source file checksums
    #[group(required = false, multiple = false)]
    Checksum {
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "checksum cannot be set directly, please use 'sysand include' and 'sysand exclude'"
        ))]
        set: Option<Infallible>,
        // Only for better error messages
        #[arg(
            hide = true,
            long,
            num_args=0,
            default_missing_value="None",
            value_parser=invalid_command(
              "checksum cannot be cleared directly, please use 'sysand exclude'"
            )
        )]
        clear: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "checksum cannot be added to directly, please use 'sysand include'"
        ))]
        add: Option<Infallible>,
        // Only for better error messages
        #[arg(hide=true, long, default_value=None, value_parser=invalid_command(
          "checksum cannot be removed from directly, please use 'sysand exclude'"
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
    GetLicence,
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
    SetLicence(String),
    SetMaintainer(Vec<String>),
    SetWebsite(String),
    SetTopic(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum ClearInfoVerb {
    ClearDescription,
    ClearLicence,
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
                _ => panic!("internal error: invalid CLI command produced"),
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
            } => pack_info(
                GetInfoVerb::GetVersion,
                set.map(SetInfoVerb::SetVersion),
                impossible(clear),
                impossible(add),
                impossible(remove),
            ),
            InfoCommand::Licence {
                set,
                clear,
                add,
                remove,
            } => pack_info(
                GetInfoVerb::GetLicence,
                set.map(SetInfoVerb::SetLicence),
                if clear {
                    Some(ClearInfoVerb::ClearLicence)
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
                set.map(SetInfoVerb::SetWebsite),
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
                clear,
                add,
                remove,
            } => pack_meta(
                GetMetaVerb::GetMetamodel,
                set.map(SetMetaVerb::SetMetamodel),
                if clear {
                    Some(ClearMetaVerb::ClearMetamodel)
                } else {
                    None
                },
                impossible(add),
                impossible(remove),
            ),
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
                clear: _,
                add: _,
                remove: _,
            } => false,
            InfoCommand::Licence {
                set: _,
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
    /// Install project in sysand_env
    Install {
        iri: String,
        version: Option<String>,
        /// Local path to interchange project
        #[arg(long, default_value = None)]
        path: Option<String>,

        #[command(flatten)]
        install_opts: InstallOptions,
        #[command(flatten)]
        dependency_opts: DependencyOptions,
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

#[derive(clap::Args, Debug, Clone)]
pub struct DependencyOptions {
    /// Use an index when resolving this usage
    #[arg(long, default_values = vec![DEFAULT_INDEX_URL], num_args=0.., help_heading = "Dependency options")]
    pub use_index: Vec<String>,
    /// Do not use any index when resolving this usage
    #[arg(
        long,
        default_value = "false",
        conflicts_with = "use_index",
        help_heading = "Dependency options"
    )]
    pub no_index: bool,
    /// Include possible usages of KerML/SysML standard libraries. By default
    /// these are excluded, as they are typically shipped with your language
    /// implementation.
    #[arg(long, default_value = "false", help_heading = "Dependency options")]
    pub include_std: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct SourcesOptions {
    #[arg(long, default_value = "false")]
    pub no_deps: bool,
    /// Include KerML/SysML standard libraries. By default
    /// these are excluded, as they are typically shipped with your language
    /// implementation.
    ///
    /// This assumes these standard libraries have been explicitly
    /// installed by sysand.
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
    #[arg(long, short, global = true, help_heading = "Global options")]
    pub no_config: bool,
    /// Give path to 'sysand.toml' to use for configuration
    #[arg(long, short, global = true, help_heading = "Global options")]
    pub config_file: Option<String>,
}

impl GlobalOptions {
    pub fn sets_log_level(&self) -> bool {
        self.verbose || self.quiet
    }
}
