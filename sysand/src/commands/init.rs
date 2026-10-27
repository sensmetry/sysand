// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::{
    CliError,
    cli::{MetamodelKindCli, MetamodelVersionCli},
    commands::info::log_license_files_note,
};
use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use clap::ValueEnum as _;
use semver::Version;
use sysand_core::{
    context::ProjectContext,
    discover::{discover_project, is_project_file},
    init::InitError,
    manifest::{
        MODEL_TOML_FILE,
        common::InheritFrom,
        edit::{push_member, validated},
        inherit::{Inherited, MetaProperty},
        workspace_table::Region,
    },
    model::{IdentifierKind, KnownMetamodel, ProjectName, ProjectPublisher},
    project::{
        local_src::LocalSrcProject,
        utils::{relativize_path, wrapfs},
    },
    utils::{RelativePathKind, parse_relative_unix_path},
    workspace::model_toml::discover_table,
};

const DEFAULT_VERSION: &str = "0.0.1";

pub fn command_init(
    name: Option<ProjectName>,
    publisher: ProjectPublisher,
    version: Option<Version>,
    license: Option<spdx::Expression>,
    metamodel: Option<KnownMetamodel>,
    path: Option<Utf8PathBuf>,
    ctx: ProjectContext,
) -> Result<()> {
    let target = match path {
        Some(p) => {
            wrapfs::create_dir_all(&p)?;
            p
        }
        None => Utf8PathBuf::from("."),
    };
    warn_parent_project(&target, &ctx)?;
    // Before `do_init`, so that a refusal is about the workspace rather
    // than about a file collision the workspace explains.
    let member = workspace_member(&target)?;
    // model-toml.md "Editing the manifest": `init` "take[s] it as a
    // mandatory argument, erroring before writing anything if it is
    // absent [...] The one exception is a project created inside a
    // workspace whose default group defines `metamodel`". The scope is
    // known by now and nothing has been written yet, which is what the
    // sentence requires.
    let metamodel = metamodel_for(metamodel, member.as_ref())?;

    let version = version.unwrap_or_else(|| Version::parse(DEFAULT_VERSION).unwrap());
    // Whether the name was chosen or inherited decides whether there is
    // anything to suggest when it turns out to be unusable.
    let named = name.is_some();
    let name = match name {
        Some(n) => n,
        None => default_name_from_path(&target)?,
    };

    let has_license = license.is_some();
    let inherited_kind = member
        .as_ref()
        .and_then(|member| member.group_metamodel.clone());
    sysand_core::init::do_init_ext(
        name.into_string(),
        publisher.into_string(),
        version.to_string(),
        license.map(|l| l.to_string()),
        Some(metamodel),
        inherited_kind,
        &mut LocalSrcProject::new_access(target, None),
    )
    .map_err(|error| match &error {
        // The name came from the directory, so the fix is not where the
        // message points. A publisher never comes from a directory.
        InitError::Identifier(identifier) if !named && identifier.kind == IdentifierKind::Name => {
            anyhow::anyhow!(
                "{error}\n\
                {SP:>8} the name came from the directory; pass `--name` to choose another",
                SP = ' '
            )
        }
        _ => anyhow::Error::new(error),
    })?;
    if has_license {
        log_license_files_note();
    }

    // After, so a project that could not be written leaves no `members`
    // entry pointing at nothing.
    if let Some(member) = member {
        append_member(&member.root, &member.path)?;
    }
    Ok(())
}

/// The metamodel `--metamodel` names: `sysml` or `kerml` the OMG
/// metamodel of `release`, anything else one of the recognized IRIs.
pub fn parse_metamodel(value: &str, release: MetamodelVersionCli) -> Result<KnownMetamodel> {
    match MetamodelKindCli::from_str(value, false) {
        Ok(kind) => Ok(KnownMetamodel::omg(kind.into(), release.into())),
        Err(_) => crate::commands::check_metamodel(value),
    }
}

/// The metamodel `init` writes: the argument, or else the default
/// group's, which is the one case where none given is not an error.
///
/// An argument that is given wins over the workspace: a group is what a
/// project inherits when it says nothing, not something that overrides
/// what it was told.
fn metamodel_for(
    metamodel: Option<KnownMetamodel>,
    member: Option<&Member>,
) -> Result<MetaProperty<String>> {
    if let Some(metamodel) = metamodel {
        return Ok(MetaProperty::Provided(metamodel.to_string()));
    }
    // Only the default group, and only for a project that is becoming a
    // member: "a named group is not guessed at either".
    if member.is_some_and(|member| member.group_metamodel.is_some()) {
        return Ok(MetaProperty::Inherited(Inherited {
            from: InheritFrom::DefaultGroup,
            at: None,
        }));
    }
    bail!(
        "`--metamodel <url>` is required: a project declares its metamodel,\n\
        {SP:>8} and it cannot be guessed. It is one of {list}\n\
        {SP:>8} (inside a workspace whose default group declares `metamodel`,\n\
        {SP:>8} the argument may be omitted and the project inherits it)",
        SP = ' ',
        list = KnownMetamodel::RECOGNIZED,
    )
}

/// What `init` learns about the workspace it is creating a project in:
/// where to append the member, and whether the default group declares a
/// `metamodel` the new project can inherit.
struct Member {
    root: Utf8PathBuf,
    path: String,
    /// The default group's `metamodel` value, when it declares one: what
    /// a marker will resolve to, and what decides the layout `init`
    /// writes (design/syside-metamodel.md, "Commands").
    group_metamodel: Option<KnownMetamodel>,
}

/// workspaces.md "Command scopes": `init` "create[s] the project and
/// append[s] it to `members`". `Ok(Some(..))` is the root to append to
/// and the path to write; `Ok(None)` is every case where nothing is
/// appended -- no workspace, or an excluded directory.
///
/// The table is read rather than the workspace loaded, so that an
/// unrelated member's mistake does not block the one command that could
/// fix it.
fn workspace_member(target: &Utf8Path) -> Result<Option<Member>> {
    // `relativize_path` asserts absolute, dot-free paths, and `target`
    // is a raw command-line argument.
    let target = wrapfs::canonicalize(target)?;
    let Some((root, table)) = discover_table(&target)? else {
        return Ok(None);
    };

    // workspaces.md "Included projects": "Workspace root must not
    // contain a project." `do_init` refuses this anyway -- the root
    // holds a `model.toml` -- so what this adds is the sentence.
    if target == root {
        bail!(
            "the workspace root must not contain a project;\n\
            {SP:>8} create it in a subdirectory",
            SP = ' '
        );
    }

    let relative = relativize_path(&target, &root)?;
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
        // "Anything under one of them is outside the workspace": a
        // standalone project, which `members` must not list.
        Region::Excluded(_) => Ok(None),
        Region::Unlisted => {
            // workspaces.md "Included projects": a project under the
            // root in neither list is a hard error. Without this the
            // author is told `model.toml` already exists, which is true
            // and says nothing about the workspace the directory sits in.
            if is_project_file(&target)? {
                bail!(
                    "the project at `{target}` lies under the workspace at `{root}`\n\
                    {SP:>8} but is neither listed in `workspace.members` nor under\n\
                    {SP:>8} `workspace.exclude`; add it to one of the two",
                    SP = ' '
                );
            }
            Ok(Some(Member {
                group_metamodel: table.default_group.metamodel.clone(),
                root,
                path: relative.to_string(),
            }))
        }
    }
}

/// Add `path` to the root's `members`, through the editor, so that the
/// rest of the file -- comments included -- is left alone.
fn append_member(root: &Utf8Path, path: &str) -> Result<()> {
    let file = root.join(MODEL_TOML_FILE);
    let text = wrapfs::read_to_string(&file)?;
    let mut document = validated(file.as_str(), &text)?;
    push_member(&mut document, path)?;
    wrapfs::write(&file, document.to_string())?;
    log::info!("added `{path}` to the workspace at `{root}`");
    Ok(())
}

fn default_name_from_path<P: AsRef<Utf8Path>>(path: P) -> Result<ProjectName> {
    let dir_name = wrapfs::canonicalize(&path)?
        .file_name()
        .ok_or_else(|| {
            CliError::InvalidDirectory(format!("path `{}` is not a directory", path.as_ref()))
        })?
        .to_owned();
    Ok(ProjectName::parse(dir_name)
        .map_err(|(dir_name, e)| CliError::InvalidDirectoryName(dir_name, e))?)
}

/// Warn about a project above `target`. A workspace above it is
/// [`workspace_member`]'s concern, which acts rather than warns.
pub fn warn_parent_project(target: &Utf8Path, ctx: &ProjectContext) -> Result<()> {
    if parse_relative_unix_path(target.as_str(), RelativePathKind::SubDirectory).is_ok() {
        // Target is a subdirectory of cwd
        if let Some(existing_project) = &ctx.current_project {
            warn_found_project(existing_project);
        }
    } else {
        // Target may be out of cwd, so cwd-derived ctx is irrelevant
        if let Some(existing_project) = discover_project(target)? {
            warn_found_project(&existing_project);
        }
    }
    Ok(())
}

fn warn_found_project(existing_project: &LocalSrcProject) {
    log::warn!(
        "found an existing project in one of target path's parent\n\
        {:>8} directories `{}`",
        ' ',
        existing_project.root_path()
    );
}
