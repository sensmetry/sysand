// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::{
    CliError,
    cli::{Dependencies, EnvProjectLocatorArgs},
};

use anstream::println;
use anyhow::{Context as _, Result, bail};
use semver::VersionReq;
use sysand_core::{
    context::ProjectContext,
    env::{local_directory::LocalDirectoryEnvironment, null::NullEnvironment},
    model::UsageRef,
    project::ProjectRead as _,
    sources::{do_sources_env, do_sources_local_src_project_no_deps, resolve_dependencies},
};

pub fn command_sources_env(
    locator: EnvProjectLocatorArgs,
    version: Option<VersionReq>,
    no_own: bool,
    dependencies: Dependencies,
    env: Option<LocalDirectoryEnvironment>,
) -> Result<()> {
    let Some(env) = env else {
        bail!("unable to identify local environment");
    };

    let project = match &locator {
        EnvProjectLocatorArgs {
            identifier: Some((publisher, name)),
            iri: None,
        } => UsageRef::Typed(publisher.as_str(), name.as_str()),
        EnvProjectLocatorArgs {
            identifier: None,
            iri: Some(iri),
        } => UsageRef::Resource(iri.borrow()),
        _ => unreachable!(),
    };

    let sources = do_sources_env(env, project, version.as_ref(), no_own, dependencies.into())?;
    for src_path in sources {
        println!("{src_path}");
    }

    Ok(())
}

pub fn command_sources_project(
    no_own: bool,
    dependencies: Dependencies,
    ctx: ProjectContext,
) -> Result<()> {
    let current_project = ctx
        .current_project
        .ok_or(CliError::MissingProjectCurrentDir)?;
    // TODO: Better bail early?
    let Some(info) = current_project.get_info()? else {
        bail!("project is missing project information")
    };
    let info = info.validate().with_context(|| {
        format!(
            "project `{}` {} has invalid metadata",
            info.name, info.version
        )
    })?;

    if !no_own {
        for src_path in do_sources_local_src_project_no_deps(&current_project, true)? {
            println!("{}", src_path);
        }
    }

    if dependencies != Dependencies::None {
        let dependencies = dependencies.into();
        let deps = if let Some(env) = ctx.env {
            resolve_dependencies(info.usage, env, dependencies)?
        } else {
            let env = NullEnvironment::new();
            resolve_dependencies(info.usage, env, dependencies)?
        };

        for dep in deps {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)? {
                println!("{}", src_path);
            }
        }
    }

    Ok(())
}

/// `sysand sources` over every member of a workspace.
///
/// workspaces.md "Command scopes": from the root, or from a non-member
/// directory under it, "all members" is the target.
///
/// Deduplicated across members, and across own sources and
/// dependencies alike. Two members sharing an external dependency
/// resolve the same installed files; and an *intra-workspace* usage
/// "resolves to the member itself, never to a copy of it from an
/// index", so a member that another member depends on has its own
/// sources reached twice. The second is the case workspaces.md calls
/// normal.
///
/// Order-preserving, because own-source order is deterministic and a
/// plain `HashSet` would randomise it. Dependency order is not
/// deterministic, which is a property of the solver rather than of this
/// loop.
pub fn command_sources_workspace(
    root: &camino::Utf8Path,
    no_own: bool,
    dependencies: Dependencies,
    ctx: ProjectContext,
) -> Result<()> {
    use std::collections::HashSet;
    use std::sync::Arc;
    use sysand_core::{
        project::local_src::LocalSrcProject,
        workspace::model_toml::{MemberChecks, Workspace},
    };

    let workspace = Workspace::load_with(root, MemberChecks::SkipStrayState)?;
    let table = Arc::new(workspace.table().clone());

    let mut seen = HashSet::new();
    let mut listed: Vec<String> = Vec::new();
    // `--only-deps` is "not the project's own sources", and at a root
    // one member's dependency is another member's tree -- so the
    // members' own sources are taken out of the union rather than
    // skipped per member, which would let them back in sideways.
    let mut own: HashSet<String> = HashSet::new();

    for member in workspace.members() {
        let project = LocalSrcProject::new_member(member.dir.clone(), None, table.clone());

        for src_path in do_sources_local_src_project_no_deps(&project, true)? {
            let rendered = src_path.to_string();
            own.insert(rendered.clone());
            if seen.insert(rendered.clone()) {
                listed.push(rendered);
            }
        }

        if dependencies != Dependencies::None {
            let Some(info) = project.get_info()? else {
                bail!("member `{}` is missing project information", member.dir)
            };
            let info = info.validate()?;
            let deps = match &ctx.env {
                // Cloned per member: `resolve_dependencies` takes the
                // environment by value. One `.sysand` for the whole
                // workspace, so every member resolves against the same
                // installed set.
                Some(env) => resolve_dependencies(info.usage, env.clone(), dependencies.into())?,
                None => {
                    resolve_dependencies(info.usage, NullEnvironment::new(), dependencies.into())?
                }
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
    }

    for src_path in listed {
        if no_own && own.contains(&src_path) {
            continue;
        }
        println!("{src_path}");
    }

    Ok(())
}
