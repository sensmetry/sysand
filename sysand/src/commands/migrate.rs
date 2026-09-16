// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use sysand_core::{
    commands::migrate::{MigrateOptions, VcsState, check_vcs, migrate_project},
    manifest::{MODEL_TOML_FILE, ModelToml, ModelTomlKind, edit::render_new},
    model::{IndexName, META_JSON_FILE, PROJECT_JSON_FILE},
    project::{
        local_src::{ProjectFormat, detect_format},
        utils::wrapfs,
    },
};

/// `sysand migrate`: a json project becomes a `model.toml` one.
pub fn command_migrate(
    path: Option<Utf8PathBuf>,
    publisher: Option<String>,
    allow_dirty: bool,
    allow_no_vcs: bool,
    cwd: &Utf8Path,
) -> Result<()> {
    let dir = path.clone().unwrap_or_else(|| cwd.to_owned());

    // Appendix A row 45. Decided here rather than through the command
    // scope: putting `migrate` in `consults_the_workspace` would make a
    // discovery error fatal, and an *unlisted* json project under a
    // workspace root is exactly the project a user wants to migrate.
    if path.is_none() && is_workspace_root(&dir)? {
        bail!("run from a workspace root, `migrate` needs the path of the json project to convert");
    }

    // The format decides what happens, but the comparison it implies
    // waits until after the working-tree check: a dirty project with a
    // mismatched pair must hear about the dirty tree first.
    let keep_existing_toml = match detect_format(&dir)? {
        ProjectFormat::Json => false,
        ProjectFormat::Toml => bail!("`{dir}` is already a `model.toml` project"),
        ProjectFormat::Neither => bail!("`{dir}` does not contain a project to migrate"),
        ProjectFormat::Both => true,
    };

    // Before anything is written: `migrate` deletes the json manifests,
    // so a user needs a way back.
    if !allow_no_vcs || !allow_dirty {
        match check_vcs(&dir)? {
            VcsState::Clean => {}
            VcsState::Dirty(paths) if !allow_dirty => {
                for changed in &paths {
                    log::debug!("changed: {changed}");
                }
                bail!(
                    "the working tree at `{dir}` has uncommitted changes; commit or stash them \
                     first\nto migrate anyway, pass `--allow-dirty`"
                );
            }
            VcsState::Dirty(_) => {}
            VcsState::NoRepository if !allow_no_vcs => bail!(
                "`{dir}` is not inside a git work tree\nto migrate anyway, pass `--allow-no-vcs`"
            ),
            VcsState::NoRepository => {}
        }
    }

    if keep_existing_toml {
        // The existing `model.toml` is kept untouched and only the json
        // files are deleted, after checking that both agree on `name`
        // and `version`.
        check_both_agree(&dir)?;
        delete_json_pair(&dir)?;
        let header = sysand_core::style::get_style_config().header;
        let migrated = "Migrated";
        log::info!(
            "{header}{migrated:>12}{header:#} `{dir}`: kept the existing `{MODEL_TOML_FILE}` \
             and removed the json manifests"
        );
        return Ok(());
    }

    let info = wrapfs::read_to_string(dir.join(PROJECT_JSON_FILE))?;
    let info = serde_json::from_str(&info)
        .map_err(|e| anyhow::anyhow!("failed to read `{PROJECT_JSON_FILE}`: {e}"))?;
    let meta = wrapfs::read_to_string(dir.join(META_JSON_FILE))?;
    let meta = serde_json::from_str(&meta)
        .map_err(|e| anyhow::anyhow!("failed to read `{META_JSON_FILE}`: {e}"))?;

    let validated = migrate_project(&info, &meta, &MigrateOptions { publisher })?;
    for warning in &validated.warnings {
        log::warn!("{warning}");
    }

    // Write first, then delete: a failed write must not leave a project
    // with no manifest at all.
    wrapfs::write(
        dir.join(MODEL_TOML_FILE),
        render_new(&validated.value).to_string(),
    )?;
    delete_json_pair(&dir)?;

    let header = sysand_core::style::get_style_config().header;
    let migrated = "Migrated";
    log::info!("{header}{migrated:>12}{header:#} `{dir}` to `{MODEL_TOML_FILE}`");
    Ok(())
}

/// Whether `dir`'s own `model.toml` defines a workspace.
fn is_workspace_root(dir: &Utf8Path) -> Result<bool> {
    let at = dir.join(MODEL_TOML_FILE);
    if !wrapfs::is_file(&at)? {
        return Ok(false);
    }
    let text = wrapfs::read_to_string(&at)?;
    Ok(matches!(
        ModelToml::peek(MODEL_TOML_FILE, &text),
        Ok(Some(ModelTomlKind::Workspace))
    ))
}

fn delete_json_pair(dir: &Utf8Path) -> Result<()> {
    for json in [PROJECT_JSON_FILE, META_JSON_FILE] {
        let at = dir.join(json);
        if wrapfs::is_file(&at)? {
            wrapfs::remove_file(&at)?;
        }
    }
    Ok(())
}

/// The `Both` case: a `model.toml` beside a json pair.
///
/// Neither side goes through `ProjectRead`: `get_project` refuses an
/// ambiguous directory before reading anything.
///
/// Only `name` and `version` are compared. Nothing checks that the two
/// manifests agree about `sources` or usages, so a `model.toml` that agrees on
/// the identity has the json project's usages deleted silently.
fn check_both_agree(dir: &Utf8Path) -> Result<()> {
    let text = wrapfs::read_to_string(dir.join(MODEL_TOML_FILE))?;
    let manifest = ModelToml::parse(MODEL_TOML_FILE, &text).map_err(|report| {
        anyhow::anyhow!("`{dir}/{MODEL_TOML_FILE}` is not a valid manifest:\n{report}")
    })?;
    let ModelToml::Project(project) = manifest.value else {
        bail!(
            "`{dir}/{MODEL_TOML_FILE}` defines a workspace, not a project, so it cannot be the \
             `model.toml` of the json project beside it"
        );
    };

    // An inherited value cannot be resolved without the workspace table,
    // which `migrate` does not load. `publisher` is refused too, though
    // the rule compares only name and version: an unresolvable publisher
    // means the two manifests may name different projects and the
    // comparison could not tell.
    let toml_version = match project.project.version.provided() {
        Some(version) => version.to_string(),
        None => bail!(
            "`{dir}/{MODEL_TOML_FILE}` inherits `version` from a workspace, so it cannot be \
             compared with the json manifest beside it"
        ),
    };
    if project.project.publisher.provided().is_none() {
        bail!(
            "`{dir}/{MODEL_TOML_FILE}` inherits `publisher` from a workspace, so it cannot be \
             compared with the json manifest beside it"
        );
    }

    let json = wrapfs::read_to_string(dir.join(PROJECT_JSON_FILE))?;
    let json: sysand_core::model::InterchangeProjectInfoRaw = serde_json::from_str(&json)
        .map_err(|e| anyhow::anyhow!("failed to read `{PROJECT_JSON_FILE}`: {e}"))?;

    // Identities compare normalized, and `IndexName` derives `PartialEq` on
    // the written string -- so `Lib test` and
    // `lib-test` are the same name and a literal comparison would call
    // them a mismatch.
    let json_name = IndexName::parse(json.name.clone())
        .map_err(|e| anyhow::anyhow!("`{PROJECT_JSON_FILE}` has an invalid `name`: {e}"))?;
    if json_name.normalized() != project.project.name.normalized() {
        bail!(
            "`{MODEL_TOML_FILE}` names `{}` but `{PROJECT_JSON_FILE}` names `{}`; \
             migrating a project that has both requires them to agree",
            project.project.name.as_str(),
            json.name
        );
    }

    // The json side is a string and the toml side a `semver::Version`,
    // so a json version that is not semver is a parse error rather than
    // a disagreement quoting a non-version.
    let json_version = semver::Version::parse(&json.version).map_err(|e| {
        anyhow::anyhow!(
            "`{PROJECT_JSON_FILE}` has version `{}`, which is not a semantic version: {e}",
            json.version
        )
    })?;
    if json_version.to_string() != toml_version {
        bail!(
            "`{MODEL_TOML_FILE}` is version `{toml_version}` but `{PROJECT_JSON_FILE}` is \
             version `{}`; migrating a project that has both requires them to agree",
            json.version
        );
    }

    Ok(())
}
