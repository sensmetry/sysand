// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    manifest::{MODEL_TOML_FILE, ModelToml, ModelTomlKind, ParsedModelToml},
    project::{
        local_src::json_manifest,
        utils::{FsIoError, wrapfs},
    },
};

/// A directory's own manifest.
#[derive(Clone, Debug)]
pub enum DirManifest {
    /// `model.toml`, read and parsed under its full path, which names it
    /// in messages.
    ModelToml {
        file: Utf8PathBuf,
        parsed: ParsedModelToml,
    },
    /// `.project.json` or `.meta.json`, and no `model.toml`. Neither is
    /// read.
    Json,
}

impl DirManifest {
    /// What `dir` holds: its `model.toml`, read once and parsed, or else
    /// the presence of a json manifest. `None` when it holds neither.
    pub fn read(dir: &Utf8Path) -> Result<Option<Self>, Box<FsIoError>> {
        let file = dir.join(MODEL_TOML_FILE);
        if wrapfs::is_file(&file)? {
            let text = wrapfs::read_to_string(&file)?;
            let parsed = ModelToml::parse_with_kind(file.as_str(), &text);
            return Ok(Some(Self::ModelToml { file, parsed }));
        }
        Ok(json_manifest(dir)?.map(|_| Self::Json))
    }

    /// Whether the directory is a project directory.
    ///
    /// A json project is one that holds either of its two manifests. A
    /// workspace root defines no project. A `model.toml` that is not TOML,
    /// or that defines neither table, counts as a project, so the error is
    /// not buried by walking past it.
    pub fn is_project(&self) -> bool {
        self.kind() != Some(ModelTomlKind::Workspace)
    }

    /// Which table the `model.toml` names; `None` for a json project.
    pub fn kind(&self) -> Option<ModelTomlKind> {
        match self {
            Self::ModelToml { file: _, parsed } => Some(parsed.kind),
            Self::Json => None,
        }
    }
}

/// The manifests of a directory and its ancestors, each read once by one
/// upward walk over canonical paths.
///
/// The walk goes as far as a workspace discovery needs: past the nearest
/// `[workspace]` to the next one above it (the outer workspace that may
/// `exclude` it), and no further than a `model.toml` that is not TOML,
/// which ends any walk to a workspace. With [`Seek::ProjectToo`] it also
/// goes on until it has found a project.
#[derive(Debug)]
pub struct Ancestry {
    /// The nearest project directory, spelled as the starting path spells
    /// it when that names the same directory.
    project: Option<Utf8PathBuf>,
    /// The position of the project's directory in `found`.
    project_at: Option<usize>,
    /// Every directory that holds a manifest, nearest first.
    found: Vec<(Utf8PathBuf, DirManifest)>,
    /// The read that ended the walk, after the project was found.
    failed: Option<Box<FsIoError>>,
}

/// How far an [`Ancestry`] walk goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seek {
    /// As far as workspace discovery needs.
    Workspace,
    /// Also until the nearest project is found.
    ProjectToo,
}

impl Ancestry {
    /// Walk up from `from`, which is canonicalized first.
    ///
    /// A read that fails before the walk has found what it seeks is an
    /// error. With [`Seek::ProjectToo`], one that fails after the project
    /// was found is kept, and surfaces only to workspace discovery, which
    /// had not finished.
    pub fn walk(from: impl AsRef<Utf8Path>, seek: Seek) -> Result<Self, Box<FsIoError>> {
        let from = from.as_ref();
        let canonical = wrapfs::canonicalize(from)?;
        let mut ancestry = Self {
            project: None,
            project_at: None,
            found: Vec::new(),
            failed: None,
        };
        let mut workspaces = 0;
        let mut lexical = Some(from);
        let mut current: Option<&Utf8Path> = Some(&canonical);
        while let Some(dir) = current {
            let manifest = match DirManifest::read(dir) {
                Ok(manifest) => manifest,
                Err(error) if ancestry.project.is_some() => {
                    ancestry.failed = Some(error);
                    break;
                }
                Err(error) => return Err(error),
            };
            if let Some(manifest) = manifest {
                if seek == Seek::ProjectToo && ancestry.project.is_none() && manifest.is_project() {
                    ancestry.project = Some(spelled(lexical, dir));
                    ancestry.project_at = Some(ancestry.found.len());
                }
                let kind = manifest.kind();
                ancestry.found.push((dir.to_owned(), manifest));
                if kind == Some(ModelTomlKind::Workspace) {
                    workspaces += 1;
                }
                let workspace_done = workspaces == 2 || kind == Some(ModelTomlKind::NotToml);
                if workspace_done && (seek == Seek::Workspace || ancestry.project.is_some()) {
                    break;
                }
            }
            lexical = lexical.and_then(lexical_parent);
            current = dir.parent();
        }
        Ok(ancestry)
    }

    /// The nearest directory at or above the start that is a project
    /// directory (see [`DirManifest::is_project`]). Always `None` for a
    /// walk that did not seek one.
    pub fn project(&self) -> Option<&Utf8Path> {
        self.project.as_deref()
    }

    /// The parse of the nearest project's `model.toml`, as the walk read it.
    /// `None` for a json project or when the walk found no project.
    pub fn project_model_toml(&self) -> Option<&ParsedModelToml> {
        match self.found.get(self.project_at?)? {
            (_, DirManifest::ModelToml { file: _, parsed }) => Some(parsed),
            (_, DirManifest::Json) => None,
        }
    }

    /// The directories that hold a manifest, nearest first, and the read
    /// that ended the walk early, if one failed.
    pub fn into_found(self) -> (Vec<(Utf8PathBuf, DirManifest)>, Option<Box<FsIoError>>) {
        (self.found, self.failed)
    }
}

#[cfg(test)]
#[path = "./discover_tests.rs"]
mod tests;

/// `canonical` in the starting path's spelling, `lexical`, when that names
/// the same directory.
fn spelled(lexical: Option<&Utf8Path>, canonical: &Utf8Path) -> Utf8PathBuf {
    match lexical {
        Some(lexical) if lexical == canonical => canonical.to_owned(),
        Some(lexical) if wrapfs::canonicalize_raw(lexical).is_ok_and(|c| c == canonical) => {
            lexical.to_owned()
        }
        _ => canonical.to_owned(),
    }
}

/// The parent of `path` by dropping its last component, when that is a
/// plain name and something is left.
fn lexical_parent(path: &Utf8Path) -> Option<&Utf8Path> {
    path.file_name()?;
    path.parent().filter(|parent| !parent.as_str().is_empty())
}
