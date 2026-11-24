// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Applying a manifest edit to an in-memory project, in the format it has.

use crate::{
    manifest::{MODEL_TOML_FILE, edit::DocumentMut, kpar_view, project::ResolvedProject},
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::memory::InMemoryProject,
};

/// Run `toml` on the project's `model.toml` when it has one, else `json` on
/// its two documents. The result is stored only when the edit succeeds.
pub(crate) fn edit_in_memory<T, E>(
    project: &mut InMemoryProject,
    toml: impl FnOnce(&mut DocumentMut, &ResolvedProject) -> Result<T, E>,
    json: impl FnOnce(
        &mut InterchangeProjectInfoRaw,
        &mut InterchangeProjectMetadataRaw,
    ) -> Result<T, E>,
) -> Result<T, E> {
    if let Some(text) = project.model_toml.clone() {
        let resolved = kpar_view::resolved_of_text(MODEL_TOML_FILE, &text)
            .expect("the manifest of an edited test project resolves");
        let mut document: DocumentMut = text.parse().expect("a manifest");
        let value = toml(&mut document, &resolved)?;
        project.model_toml = Some(document.to_string());
        return Ok(value);
    }
    let info = project.info.as_mut().expect("a project with `info`");
    let meta = project.meta.as_mut().expect("a project with `meta`");
    let mut edited_info = info.clone();
    let mut edited_meta = meta.clone();
    let value = json(&mut edited_info, &mut edited_meta)?;
    *info = edited_info;
    *meta = edited_meta;
    Ok(value)
}
