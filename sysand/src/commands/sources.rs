// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::{
    CliError,
    cli::{DependenciesCli, EnvProjectLocatorArgs},
};

use anstream::println;
use anyhow::{Context as _, Result, bail};
use semver::VersionReq;
use sysand_core::{
    context::ProjectContext,
    env::{local_directory::LocalDirectoryEnvironment, null::NullEnvironment},
    model::UsageRef,
    sources::{
        do_sources_env, do_sources_local_src_definition, do_sources_local_src_project_no_deps,
        resolve_dependencies,
    },
};

pub fn command_sources_env(
    locator: EnvProjectLocatorArgs,
    version: Option<VersionReq>,
    no_own: bool,
    dependencies: DependenciesCli,
    env: Option<LocalDirectoryEnvironment>,
) -> Result<()> {
    let Some(env) = env else {
        bail!("unable to identify local environment");
    };

    let project = match &locator {
        EnvProjectLocatorArgs {
            identifier: Some((publisher, name)),
            iri: None,
        } => UsageRef::Typed(publisher, name),
        EnvProjectLocatorArgs {
            identifier: None,
            iri: Some(iri),
        } => UsageRef::Resource(iri.borrow()),
        _ => unreachable!(),
    };

    let sources = do_sources_env(env, project, version.as_ref(), no_own, dependencies.into())?;
    for src_path in sources {
        emit_source(&src_path);
    }

    Ok(())
}

pub fn command_sources_project(
    no_own: bool,
    dependencies: DependenciesCli,
    ctx: ProjectContext,
) -> Result<()> {
    let Some(current_project) = ctx.current_project() else {
        bail!(CliError::MissingProjectCurrentDir);
    };
    // TODO: Better bail early?
    let Some(definition) = ctx.current_definition() else {
        bail!("project is missing project information")
    };
    let usages = definition.validated_usages().with_context(|| {
        format!(
            "project `{}` {} has invalid metadata",
            definition.name(),
            definition.version()
        )
    })?;

    if !no_own {
        for src_path in do_sources_local_src_definition(current_project, definition, true)? {
            emit_source(&src_path);
        }
    }

    if dependencies != DependenciesCli::None {
        let dependencies = dependencies.into();
        let deps = if let Some(env) = ctx.env {
            resolve_dependencies(usages, env, dependencies)?
        } else {
            let env = NullEnvironment::new();
            resolve_dependencies(usages, env, dependencies)?
        };

        for dep in deps {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)? {
                emit_source(&src_path);
            }
        }
    }

    Ok(())
}

/// `sysand sources` over every member of a workspace.
///
/// From the root, or from a non-member directory under it, all members
/// are the target.
///
/// Deduplicated across members, and across own sources and dependencies
/// alike: members sharing a dependency resolve the same installed files,
/// and a member that another member depends on has its own sources
/// reached twice.
///
/// Own-source order is deterministic; dependency order is up to the
/// solver.
pub fn command_sources_workspace(
    no_own: bool,
    dependencies: DependenciesCli,
    ctx: ProjectContext,
) -> Result<()> {
    use std::collections::HashSet;
    use sysand_core::project::{ProjectDefinition, local_src::LocalSrcProject};

    let workspace = ctx.workspace().expect("a workspace root means a workspace");

    // A group entry's path is relative to the workspace root, which is not
    // known below, so resolving it would look in the member's directory.
    for member in workspace.members() {
        for located in &member.project.usages {
            if located.base != sysand_core::manifest::usage::UsageBase::Project
                && let Some(path) = located.usage.local_path()
            {
                bail!(
                    "member `{}` inherits the usage `{}`, whose path `{path}` is relative to \
                     the workspace root; listing the sources of a project that inherits a path \
                     usage is not supported yet",
                    member.path.raw,
                    located.usage
                );
            }
        }
    }

    let table = workspace.table().clone();

    let mut seen = HashSet::new();
    let mut listed: Vec<String> = Vec::new();
    // For `--only-deps`, the members' own sources are removed from the
    // union at the end: skipping them per member would let one member's
    // sources back in as another's dependency.
    let mut own: HashSet<String> = HashSet::new();
    // Every member's usages, solved together as the workspace lock solves
    // them.
    let mut usages = Vec::new();

    for member in workspace.members() {
        let project = LocalSrcProject::new_member(member.dir.clone(), None, table.clone());

        for source in member.project.sources.iter() {
            let rendered = project.get_source_path(source)?.to_string();
            own.insert(rendered.clone());
            if seen.insert(rendered.clone()) {
                listed.push(rendered);
            }
        }

        if dependencies != DependenciesCli::None {
            usages.extend(
                ProjectDefinition::Toml(Box::new(member.project.clone())).validated_usages()?,
            );
        }
    }

    if dependencies != DependenciesCli::None {
        let deps = match ctx.env {
            Some(env) => resolve_dependencies(usages, env, dependencies.into())?,
            None => resolve_dependencies(usages, NullEnvironment::new(), dependencies.into())?,
        };
        for dep in deps {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)? {
                let rendered = src_path.to_string();
                if seen.insert(rendered.clone()) {
                    listed.push(rendered);
                }
            }
        }
    }

    for src_path in listed {
        if no_own && own.contains(&src_path) {
            continue;
        }
        emit_source(&src_path);
    }

    Ok(())
}

/// One source path: a line in text mode, an entry of the `sources`
/// document under `--format json`, in the same order.
fn emit_source(path: &impl std::fmt::Display) {
    if crate::output::is_json() {
        crate::output::push("sources", path.to_string());
    } else {
        println!("{path}");
    }
}
