// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, io::ErrorKind};

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

/// The manifests of directories already read, for walks that start in
/// directories with ancestors in common.
///
/// A walk reads each directory's manifest through the cache, so one that is
/// read again is not parsed again. The cache is keyed by path: each `dir`
/// must be canonical, so that one directory has one key, and the files must
/// not change while the cache is in use.
#[derive(Debug, Default)]
pub struct ManifestCache {
    read: HashMap<Utf8PathBuf, Option<DirManifest>>,
}

impl ManifestCache {
    /// What the canonical `dir` holds: its parsed `model.toml`, or else the
    /// presence of a json manifest. A directory read before is answered
    /// from the cache.
    pub fn read(&mut self, dir: &Utf8Path) -> Result<Option<DirManifest>, Box<FsIoError>> {
        debug_assert!(dir.is_absolute(), "`{dir}` is not canonical");
        if let Some(read) = self.read.get(dir) {
            return Ok(read.clone());
        }
        let read = DirManifest::read(dir)?;
        self.read.insert(dir.to_owned(), read.clone());
        Ok(read)
    }
}

/// The manifests of a directory and its ancestors, each read once by one
/// upward walk over canonical paths.
///
/// The walk goes as far as a workspace discovery needs: past the nearest
/// `[workspace]` to the next one above it (the outer workspace that may
/// `exclude` it), and no further than a `model.toml` that is not TOML,
/// which ends any walk to a workspace. With [`Seek::ProjectToo`] it also
/// goes on until it has found a project, and with [`Seek::ProjectOnly`] it
/// stops there.
#[derive(Debug)]
pub struct Ancestry {
    /// The starting path, canonicalized.
    start: Utf8PathBuf,
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
    /// Only until the nearest project is found: no enclosing workspace is
    /// read.
    ProjectOnly,
}

impl Ancestry {
    /// Walk up from `from`, which is canonicalized first, so a `from` that
    /// does not exist is an error.
    ///
    /// A read that fails before the walk has found what it seeks is an
    /// error. With [`Seek::ProjectToo`], one that fails after the project
    /// was found is kept, and surfaces only to workspace discovery, which
    /// had not finished.
    pub fn walk(from: impl AsRef<Utf8Path>, seek: Seek) -> Result<Self, Box<FsIoError>> {
        Self::walk_cached(from, seek, &mut ManifestCache::default())
    }

    /// Walk up from `from`, which is canonicalized first, reading each
    /// directory's manifest once per `cache`.
    pub fn walk_cached(
        from: impl AsRef<Utf8Path>,
        seek: Seek,
        cache: &mut ManifestCache,
    ) -> Result<Self, Box<FsIoError>> {
        let from = from.as_ref();
        Self::walk_from(wrapfs::canonicalize(from)?, from, seek, cache)
    }

    /// [`Self::walk`] from a directory that may not exist yet, such as one
    /// about to be created: when `from` does not exist, its nearest
    /// existing ancestor is canonicalized and the rest appended. The
    /// missing directories hold no manifests.
    pub fn walk_maybe_missing(
        from: impl AsRef<Utf8Path>,
        seek: Seek,
    ) -> Result<Self, Box<FsIoError>> {
        let from = from.as_ref();
        Self::walk_from(resolve(from)?, from, seek, &mut ManifestCache::default())
    }

    /// The walk up from `canonical`, which `from` spells.
    fn walk_from(
        canonical: Utf8PathBuf,
        from: &Utf8Path,
        seek: Seek,
        cache: &mut ManifestCache,
    ) -> Result<Self, Box<FsIoError>> {
        let mut ancestry = Self {
            start: canonical.clone(),
            project: None,
            project_at: None,
            found: Vec::new(),
            failed: None,
        };
        let mut workspaces = 0;
        let mut lexical = Some(from);
        let mut current: Option<&Utf8Path> = Some(&canonical);
        while let Some(dir) = current {
            let manifest = match cache.read(dir) {
                Ok(manifest) => manifest,
                Err(error) if ancestry.project.is_some() => {
                    ancestry.failed = Some(error);
                    break;
                }
                Err(error) => return Err(error),
            };
            if let Some(manifest) = manifest {
                if seek != Seek::Workspace && ancestry.project.is_none() && manifest.is_project() {
                    ancestry.project = Some(spelled(lexical, dir));
                    ancestry.project_at = Some(ancestry.found.len());
                }
                let kind = manifest.kind();
                ancestry.found.push((dir.to_owned(), manifest));
                if kind == Some(ModelTomlKind::Workspace) {
                    workspaces += 1;
                }
                if seek == Seek::ProjectOnly && ancestry.project.is_some() {
                    break;
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

    /// The starting path, resolved.
    pub fn start(&self) -> &Utf8Path {
        &self.start
    }

    /// Whether the starting directory holds a workspace `model.toml`.
    pub fn starts_at_workspace_root(&self) -> bool {
        self.found.first().is_some_and(|(dir, manifest)| {
            *dir == self.start && manifest.kind() == Some(ModelTomlKind::Workspace)
        })
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

    /// The manifest of the starting directory itself, as the walk read it.
    /// `None` when it holds none.
    pub fn start_manifest(&self) -> Option<&DirManifest> {
        self.found
            .first()
            .filter(|(dir, _)| *dir == self.start)
            .map(|(_, manifest)| manifest)
    }

    /// The directories that hold a manifest, nearest first, and the read
    /// that ended the walk early, if one failed.
    pub fn into_found(self) -> (Vec<(Utf8PathBuf, DirManifest)>, Option<Box<FsIoError>>) {
        (self.found, self.failed)
    }
}

/// `path` canonicalized, or, when it does not exist, its nearest existing
/// ancestor canonicalized with the missing components appended. A missing
/// component that is not a plain name (`..`) cannot be resolved, and is the
/// canonicalization error.
fn resolve(path: &Utf8Path) -> Result<Utf8PathBuf, Box<FsIoError>> {
    let not_found = match wrapfs::canonicalize_raw(path) {
        Ok(canonical) => return Ok(canonical),
        Err(error) if error.kind() == ErrorKind::NotFound => error,
        Err(error) => return Err(Box::new(FsIoError::Canonicalize(path.into(), error))),
    };
    let absolute = wrapfs::absolute(path)?;
    let mut missing = Vec::new();
    let mut existing = absolute.as_path();
    loop {
        let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
            return Err(Box::new(FsIoError::Canonicalize(path.into(), not_found)));
        };
        missing.push(name);
        existing = parent;
        match wrapfs::canonicalize_raw(existing) {
            Ok(mut resolved) => {
                resolved.extend(missing.iter().rev());
                return Ok(resolved);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(Box::new(FsIoError::Canonicalize(existing.into(), error))),
        }
    }
}

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

#[cfg(test)]
#[path = "./discover_tests.rs"]
mod tests;
