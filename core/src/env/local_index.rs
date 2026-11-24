// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! A Sysand index read from a directory on this machine, in the layout
//! `sysand index init`/`add` write (`design/index-protocol.md` "Filesystem
//! indexes"): `index.json`, `<publisher>/<name>/versions.json` and
//! `<publisher>/<name>/<version>/project.kpar`; the same bytes an HTTP
//! index serves, read with `std::fs`.
//!
//! The directory must hold `index.json`; it is checked once, on first use.
//!
//! Every path the index names (an archive under a version directory,
//! `versions.json`, an enumerated `*.kpar`) is canonicalized and must
//! stay under the canonicalized root: the directory is trusted no more
//! than an HTTP server, and `..` or a symlink must not turn a dependency
//! solve into a read elsewhere.
//!
//! There is no discovery step (`sysand-index-config.json` is never read:
//! the root *is* `index_root`, and a directory has no `api_root`) and no
//! authentication. Blocking `std::fs` calls inside the async trait
//! methods are accepted: every read is a small JSON document or an
//! archive header, the same cost the synchronous
//! [`super::local_directory::LocalDirectoryEnvironment`] pays.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use camino::{Utf8Path, Utf8PathBuf};
use semver::Version;
use thiserror::Error;

use crate::{
    commands::index::{INDEX_FILE_NAME, KPAR_FILE_NAME, VERSIONS_FILE_NAME},
    env::ReadEnvironmentAsync,
    index::{
        iri::{ParseIriError, parse_iri},
        model::{
            IndexJson, ProjectStatus, VersionStatus, VersionsJson, VersionsJsonError,
            validate_versions,
        },
    },
    project::{
        AsAsyncProject, KparMeta, ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParError, LocalKParProject},
        utils::{FsIoError, wrapfs},
    },
};

#[derive(Error, Debug)]
pub enum LocalIndexError {
    /// The directory exists but has no `index.json`.
    #[error("`{path}` is not a Sysand index: it has no `{INDEX_FILE_NAME}`")]
    NotAnIndex { path: Utf8PathBuf },
    #[error("index `{path}` is not a directory")]
    NotADirectory { path: Utf8PathBuf },
    #[error("`{entry}` in `{file}` escapes the index at `{root}`")]
    EscapesIndex {
        entry: Utf8PathBuf,
        file: Utf8PathBuf,
        root: Utf8PathBuf,
    },
    #[error("failed to parse JSON from `{file}`")]
    Json {
        file: Utf8PathBuf,
        #[source]
        source: serde_json::Error,
    },
    /// The `versions.json` shape rules, shared with the HTTP client.
    #[error(transparent)]
    Versions(#[from] VersionsJsonError),
    #[error("versions.json at `{file}` does not list version `{version}`")]
    VersionNotInIndex { file: Utf8PathBuf, version: String },
    #[error("project `{iri}` is not in this index (no versions.json at `{file}`)")]
    ProjectNotInIndex { file: Utf8PathBuf, iri: String },
    #[error(
        "version `{version}` of `{iri}` was removed (versions.json at `{file}` marks it `status: \"removed\"`)"
    )]
    VersionRemoved {
        file: Utf8PathBuf,
        iri: String,
        version: String,
    },
    #[error(transparent)]
    MalformedIri(#[from] ParseIriError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    Kpar(#[from] Box<LocalKParError>),
}

impl From<LocalKParError> for LocalIndexError {
    fn from(value: LocalKParError) -> Self {
        Self::Kpar(Box::new(value))
    }
}

/// One `versions.json` entry after validation: what a candidate needs to be
/// enumerated and then opened.
#[derive(Debug, Clone)]
struct Candidate {
    version: Version,
    status: VersionStatus,
    /// The digest the entry advertises, verified against the archive when
    /// it is opened.
    expected: KparMeta,
}

/// The versions of one identity, newest first.
type Candidates = Arc<[Candidate]>;

/// A structured index read from `root`.
#[derive(Debug)]
pub struct LocalIndexEnvironment {
    /// The spelling the user gave, for messages.
    display: Utf8PathBuf,
    /// Absolute, as spelled (symlinks intact); what paths are built from.
    root: Utf8PathBuf,
    /// The canonical root, once the directory has been checked to be an
    /// index; populated on first access.
    checked: Mutex<Option<Arc<Utf8PathBuf>>>,
    /// Validated `versions.json` documents by IRI.
    versions_cache: Mutex<HashMap<String, Candidates>>,
}

impl LocalIndexEnvironment {
    /// Point at `root` without reading it; a relative path is resolved
    /// against the current directory now, so the environment does not
    /// depend on a later `chdir`. The directory is read on first use.
    pub fn new(root: impl Into<Utf8PathBuf>) -> Self {
        let display = root.into();
        // `absolute` fails only when the current directory is gone; the
        // path as spelled then fails the same way on first use, with a
        // message naming it.
        let root = wrapfs::absolute(&display).unwrap_or_else(|_| display.clone());
        Self {
            display,
            root,
            checked: Mutex::new(None),
            versions_cache: Mutex::new(HashMap::new()),
        }
    }

    /// The canonical root, after checking that the directory is an index.
    fn canonical_root(&self) -> Result<Arc<Utf8PathBuf>, LocalIndexError> {
        let mut guard = self
            .checked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(found) = guard.as_ref() {
            return Ok(Arc::clone(found));
        }
        if !wrapfs::is_dir(&self.root)? {
            return Err(LocalIndexError::NotADirectory {
                path: self.display.clone(),
            });
        }
        if !wrapfs::is_file(self.root.join(INDEX_FILE_NAME))? {
            return Err(LocalIndexError::NotAnIndex {
                path: self.display.clone(),
            });
        }
        let found = Arc::new(wrapfs::canonicalize(&self.root)?);
        *guard = Some(Arc::clone(&found));
        Ok(found)
    }

    /// Canonicalize `candidate` (following symlinks) and require it to
    /// stay under the index root; `file` is what named it, for the
    /// message. Returns the candidate as named, not canonicalized, so
    /// messages downstream keep the user's spelling.
    fn contained(
        &self,
        canonical_root: &Utf8Path,
        candidate: &Utf8Path,
        file: &Utf8Path,
    ) -> Result<Utf8PathBuf, LocalIndexError> {
        let canonical = wrapfs::canonicalize(candidate)?;
        if canonical.starts_with(canonical_root) {
            Ok(candidate.to_path_buf())
        } else {
            Err(LocalIndexError::EscapesIndex {
                entry: candidate.to_path_buf(),
                file: file.to_path_buf(),
                root: self.display.clone(),
            })
        }
    }

    /// `<root>/<publisher>/<name>` (or `<root>/_iri/<sha256>`), the
    /// project directory.
    fn project_dir(&self, iri: &str) -> Result<Utf8PathBuf, LocalIndexError> {
        let [first, second] = parse_iri(iri)?.get_path_segments();
        Ok(self.root.join(first).join(second))
    }

    /// The validated `versions.json` of `iri`, or `None`
    /// when the file is absent (the project is not in this index, as for a
    /// 404 over HTTP).
    fn versions_of(
        &self,
        canonical_root: &Utf8Path,
        iri: &str,
    ) -> Result<Option<Candidates>, LocalIndexError> {
        if let Some(found) = self
            .versions_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(iri)
        {
            return Ok(Some(Arc::clone(found)));
        }
        let file = self.project_dir(iri)?.join(VERSIONS_FILE_NAME);
        if !wrapfs::is_file(&file)? {
            return Ok(None);
        }
        self.contained(canonical_root, &file, &file)?;
        let text = wrapfs::read_to_string(&file)?;
        let document: VersionsJson =
            serde_json::from_str(&text).map_err(|source| LocalIndexError::Json {
                file: file.clone(),
                source,
            })?;
        let validated: Candidates = validate_versions(file.as_str(), document)?
            .into_iter()
            .map(|advertised| Candidate {
                version: advertised.version,
                status: advertised.status,
                expected: KparMeta {
                    size_bytes: advertised.kpar_size,
                    sha256_hex: advertised.kpar_digest.as_hex().to_owned(),
                },
            })
            .collect();
        self.versions_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(iri.to_owned(), Arc::clone(&validated));
        Ok(Some(validated))
    }

    fn candidates(&self, iri: &str) -> Result<Option<Candidates>, LocalIndexError> {
        let canonical_root = self.canonical_root()?;
        self.versions_of(&canonical_root, iri)
    }
}

type ResultStream<T> = futures::stream::Iter<std::vec::IntoIter<Result<T, LocalIndexError>>>;

impl ReadEnvironmentAsync for LocalIndexEnvironment {
    type ReadError = LocalIndexError;

    type UriStream = ResultStream<String>;

    async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
        self.canonical_root()?;
        let file = self.root.join(INDEX_FILE_NAME);
        let text = wrapfs::read_to_string(&file)?;
        let index: IndexJson =
            serde_json::from_str(&text).map_err(|source| LocalIndexError::Json { file, source })?;
        let iris: Vec<Result<String, LocalIndexError>> = index
            .projects
            .into_iter()
            .filter(|p| p.status == ProjectStatus::Available)
            .map(|p| Ok(p.iri))
            .collect();
        Ok(futures::stream::iter(iris))
    }

    type VersionStream = ResultStream<String>;

    async fn versions_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Self::VersionStream, Self::ReadError> {
        let Some(candidates) = self.candidates(uri.as_ref())? else {
            log::debug!(
                "no versions for `{iri}` in filesystem index `{root}`: yielding empty candidate stream",
                iri = uri.as_ref(),
                root = self.display,
            );
            return Ok(futures::stream::iter(vec![]));
        };
        // Retired entries are excluded from new resolutions.
        let versions: Vec<Result<String, LocalIndexError>> = candidates
            .iter()
            .filter(|c| c.status == VersionStatus::Available)
            .map(|c| Ok(c.version.to_string()))
            .collect();
        Ok(futures::stream::iter(versions))
    }

    type InterchangeProjectRead = AsAsyncProject<LocalKParProject>;

    async fn get_project_async<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        let iri = uri.as_ref();
        let version = version.as_ref();
        let canonical_root = self.canonical_root()?;
        let versions_file = self.project_dir(iri)?.join(VERSIONS_FILE_NAME);
        let candidates =
            self.candidates(iri)?
                .ok_or_else(|| LocalIndexError::ProjectNotInIndex {
                    file: versions_file.clone(),
                    iri: iri.to_owned(),
                })?;
        let not_listed = || LocalIndexError::VersionNotInIndex {
            file: versions_file.clone(),
            version: version.to_owned(),
        };
        // A non-semver request is simply not listed.
        let requested = Version::parse(version).map_err(|_parse| not_listed())?;
        let candidate = candidates
            .iter()
            .find(|c| c.version == requested)
            .ok_or_else(not_listed)?;
        if candidate.status == VersionStatus::Removed {
            return Err(LocalIndexError::VersionRemoved {
                file: versions_file,
                iri: iri.to_owned(),
                version: candidate.version.to_string(),
            });
        }
        let named = self
            .project_dir(iri)?
            .join(candidate.version.to_string())
            .join(KPAR_FILE_NAME);
        let archive = self.contained(&canonical_root, &named, &versions_file)?;
        // The archive's declared identity is not checked against `iri` here.
        let project = LocalKParProject::new_with_expected(
            archive,
            KparInnerPath::Root,
            None,
            candidate.expected.clone(),
        );
        Ok(project.to_async())
    }
}

#[cfg(test)]
#[path = "./local_index_tests.rs"]
mod tests;
