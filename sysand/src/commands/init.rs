// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::{
    CliError,
    cli::{MetamodelKindCli, MetamodelVersionCli},
    commands::info::log_license_files_note,
};
use std::sync::Arc;

use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use clap::ValueEnum as _;
use semver::Version;
use sysand_core::paths::relativize_path;
use sysand_core::{
    context::{AddMemberError, ProjectContext},
    init::{
        GivenProject, InheritableKey, InitError, InitProject, ProjectKind, RequiredKey,
        default_version, do_init_json, do_init_toml, plan_init_toml,
    },
    manifest::workspace_config::Region,
    model::{IdentifierKind, KnownMetamodel, ProjectName, ProjectPublisher},
    project::local_src::{LocalSrcError, LocalSrcProject},
};

pub fn command_init(
    name: Option<ProjectName>,
    publisher: Option<ProjectPublisher>,
    version: Option<Version>,
    license: Option<spdx::Expression>,
    kind: ProjectKind,
    metamodel: Option<KnownMetamodel>,
    mut ctx: ProjectContext,
) -> Result<()> {
    // Resolved, and created by the first write.
    let target = ctx.current_directory.clone();
    if let Some(existing_project) = ctx.current_project() {
        warn_found_project(existing_project);
    }
    // Whether the name was chosen or inherited decides whether there is
    // anything to suggest when it turns out to be unusable.
    let named = name.is_some();
    let name = match name {
        Some(n) => n,
        None => default_name_from_path(&target)?,
    };

    // An inherited license gets no license-files note: its license files
    // belong to the workspace.
    let has_license = license.is_some();
    init_project(
        &mut ctx,
        &target,
        InitArgs {
            name,
            publisher,
            version,
            license,
            metamodel,
            kind,
        },
    )
    .map_err(|failure| init_failure(named, failure))?;
    if has_license {
        log_license_files_note();
    }

    Ok(())
}

/// What a project is created from.
#[derive(Debug)]
pub struct InitArgs {
    pub name: ProjectName,
    pub publisher: Option<ProjectPublisher>,
    pub version: Option<Version>,
    pub license: Option<spdx::Expression>,
    pub metamodel: Option<KnownMetamodel>,
    pub kind: ProjectKind,
}

/// Why [`init_project`] failed.
#[derive(Debug)]
pub enum InitFailure {
    /// The project could not be created, or could not be written.
    Init(InitError<LocalSrcError>),
    /// The directory is not somewhere the workspace allows a project, or
    /// the workspace would not hold together with it.
    Refused(anyhow::Error),
    /// A `.project.json` project must be given what it cannot inherit.
    JsonNeeds(RequiredKey),
}

/// Create the project `args` describe in `target`, and append it to the
/// `members` of the workspace `ctx` holds, if `target` is to be a member.
///
/// A member is checked with the workspace before anything is written, and
/// inherits the keys it is not given from the workspace.
pub fn init_project(
    ctx: &mut ProjectContext,
    target: &Utf8Path,
    args: InitArgs,
) -> Result<(), InitFailure> {
    // Before `do_init_toml`: a refusal should be about the workspace, not a file
    // collision the workspace explains.
    let member = workspace_member(target, ctx).map_err(InitFailure::Refused)?;
    let InitArgs {
        name,
        publisher,
        version,
        license,
        metamodel,
        kind,
    } = args;
    let mut storage = LocalSrcProject::new_access(target.to_owned(), None);
    match kind {
        ProjectKind::ModelToml => {
            // An excluded directory holds a standalone project, which
            // inherits nothing.
            let config = member
                .as_ref()
                .and_then(|_| ctx.workspace())
                .map(|workspace| Arc::clone(workspace.config()));
            let init = InitProject {
                name,
                publisher,
                version,
                license,
                metamodel,
                workspace: config.as_deref(),
            };
            match &member {
                None => do_init_toml(init, &mut storage).map_err(InitFailure::Init),
                // The workspace must load with the project before either
                // is written.
                Some(member) => {
                    let plan = plan_init_toml(init).map_err(InitFailure::Init)?;
                    let text = plan.text();
                    ctx.add_member(&member.path, &text, || plan.write(&mut storage))
                        .map_err(|error| match error {
                            AddMemberError::Member(error) => InitFailure::Init(error),
                            error => InitFailure::Refused(anyhow::Error::new(error)),
                        })?;
                    log::info!(
                        "added `{}` to the workspace at `{}`",
                        member.path,
                        member.root
                    );
                    Ok(())
                }
            }
        }
        // A `.meta.json` cannot hold the marker that inherits a key.
        ProjectKind::ProjectJson => {
            if let Some(member) = member {
                return Err(InitFailure::Refused(anyhow::anyhow!(
                    "`{}` would be a member of the workspace at `{}`, \
                    and a workspace member must be a `model.toml` project",
                    member.path,
                    member.root,
                )));
            }
            let Some(metamodel) = metamodel else {
                return Err(InitFailure::JsonNeeds(RequiredKey::Metamodel));
            };
            let Some(publisher) = publisher else {
                return Err(InitFailure::JsonNeeds(RequiredKey::Publisher));
            };
            let project = GivenProject {
                name,
                publisher,
                version: version.unwrap_or_else(default_version),
                license,
                metamodel,
            };
            do_init_json(project, &mut storage).map_err(InitFailure::Init)
        }
    }
}

/// An init failure, spelled for the command line.
///
/// `named` is whether the name was given, as opposed to taken from the
/// directory.
fn init_failure(named: bool, failure: InitFailure) -> anyhow::Error {
    match failure {
        InitFailure::Init(error) => init_error(named, error),
        InitFailure::Refused(error) => error,
        InitFailure::JsonNeeds(RequiredKey::Metamodel) => anyhow::anyhow!(
            "`--metamodel <url>` is required for `--kind project-json`: a project\n\
            {SP:>8} declares its metamodel, and a `.meta.json` cannot inherit it.\n\
            {SP:>8} It is one of {list}",
            SP = ' ',
            list = KnownMetamodel::RECOGNIZED,
        ),
        InitFailure::JsonNeeds(RequiredKey::Publisher) => anyhow::anyhow!(
            "`--publisher <PUBLISHER>` is required for `--kind project-json`:\n\
            {SP:>8} a `.project.json` cannot inherit it",
            SP = ' ',
        ),
    }
}

/// The metamodel `--metamodel` names: `sysml` or `kerml` the OMG
/// metamodel of `release`, anything else one of the recognized IRIs.
pub fn parse_metamodel(value: &str, release: MetamodelVersionCli) -> Result<KnownMetamodel> {
    match MetamodelKindCli::from_str(value, false) {
        Ok(kind) => Ok(KnownMetamodel::omg(kind.into(), release.into())),
        // ast-grep-ignore: empty-err-arm
        Err(_) => crate::commands::check_metamodel(value),
    }
}

/// Where `init` appends the project it creates in a workspace.
struct Member {
    root: Utf8PathBuf,
    path: String,
}

/// `init` in a workspace creates the project and appends it to
/// `members`. `Ok(Some(..))` is the root to append to and the path to
/// write; `Ok(None)` is every case where nothing is appended: no
/// workspace, or an excluded directory.
fn workspace_member(target: &Utf8Path, ctx: &ProjectContext) -> Result<Option<Member>> {
    let Some(workspace) = ctx.workspace() else {
        return Ok(None);
    };
    let (root, table) = (workspace.root(), workspace.config());
    // `do_init_toml` would refuse this anyway as the root holds a `model.toml`;
    // this gives the reason.
    if target == root {
        bail!(
            "the workspace root must not contain a project;\n\
            {SP:>8} create it in a subdirectory",
            SP = ' '
        );
    }

    let relative = relativize_path(target, root)?;
    match table.region(&relative) {
        Region::Member(_, entry) if relative == entry.path => bail!(
            "`{relative}` is already member `{}`;\n\
            {SP:>8} a member directory holds one project",
            entry.raw,
            SP = ' '
        ),
        Region::Member(_, entry) => bail!(
            "`{relative}` lies inside member `{}`;\n\
            {SP:>8} nested members are not supported",
            entry.raw,
            SP = ' '
        ),
        // Anything under an excluded directory is outside the workspace:
        // a standalone project, which `members` must not list.
        Region::Excluded(_) => Ok(None),
        Region::Unlisted => Ok(Some(Member {
            root: root.to_owned(),
            path: relative.to_string(),
        })),
    }
}

/// An init failure, spelled for the command line.
///
/// `named` is whether the name was given, as opposed to taken from the
/// directory.
fn init_error(named: bool, error: InitError<LocalSrcError>) -> anyhow::Error {
    match error {
        // The name came from the directory, so the fix is not where the
        // message points. A publisher never comes from a directory.
        InitError::Identifier(identifier) if !named && identifier.kind == IdentifierKind::Name => {
            CliError::InvalidDirectoryIdentifier(identifier).into()
        }
        InitError::Missing(RequiredKey::Metamodel) => anyhow::anyhow!(
            "`--metamodel <url>` is required: a project declares its metamodel,\n\
            {SP:>8} and it cannot be guessed. It is one of {list}\n\
            {SP:>8} (inside a workspace where one group declares `metamodel`,\n\
            {SP:>8} the argument may be omitted and the project inherits it)",
            SP = ' ',
            list = KnownMetamodel::RECOGNIZED,
        ),
        InitError::Missing(RequiredKey::Publisher) => anyhow::anyhow!(
            "`--publisher <PUBLISHER>` is required\n\
            {SP:>8} (inside a workspace where one group declares `publisher`,\n\
            {SP:>8} the argument may be omitted and the project inherits it)",
            SP = ' ',
        ),
        error @ InitError::AmbiguousGroup { key, groups: _ } => {
            let argument = match key {
                InheritableKey::Publisher => "--publisher <PUBLISHER>",
                InheritableKey::Version => "--version <VERSION>",
                InheritableKey::License => "--license <LICENSE>",
                InheritableKey::Metamodel => "--metamodel <url>",
            };
            anyhow::anyhow!(
                "{error};\n\
                give `{argument}` (to inherit from one of the groups, write\n\
                `{key}.workspace = \"<group-name>\"` in `model.toml` afterwards)"
            )
        }
        error => anyhow::Error::new(error),
    }
}

/// The name of `path`, which is resolved.
fn default_name_from_path(path: &Utf8Path) -> Result<ProjectName> {
    let dir_name = path
        .file_name()
        .ok_or_else(|| CliError::InvalidDirectory(format!("path `{path}` is not a directory")))?
        .to_owned();
    Ok(ProjectName::parse(dir_name)
        .map_err(|(dir_name, e)| CliError::InvalidDirectoryName(dir_name, e))?)
}

fn warn_found_project(existing_project: &LocalSrcProject) {
    log::warn!(
        "found an existing project in one of target path's parent\n\
        {:>8} directories `{}`",
        ' ',
        existing_project.root_path()
    );
}
