// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! A Sysand index read from a directory on this machine, in either of the
//! two shapes `design/index-protocol.md` "Filesystem indexes" names:
//!
//! - **structured**: the layout `sysand index init`/`add` write --
//!   `index.json`, `<publisher>/<name>/versions.json` and
//!   `<publisher>/<name>/<version>/project.kpar`; the same bytes an HTTP
//!   index serves, read with `std::fs` instead of `reqwest`;
//! - **folder**: a bare directory of `*.kpar` files with no
//!   metadata; each archive's `.project.json` supplies `publisher`, `name`
//!   and `version`, and the environment synthesizes the `versions.json`
//!   view an index would have advertised.
//!
//! The shape is detected once, on first use: `index.json` present means
//! structured, otherwise any `*.kpar` (extension compared
//! case-insensitively) means folder, otherwise the directory is refused.
//! Neither shape is a fallback for the other and neither warns.
//!
//! Every path the index names -- an archive under a version directory,
//! `versions.json`, an enumerated `*.kpar` -- is canonicalized and must
//! stay under the canonicalized root: a directory a user was
//! handed is trusted less than an HTTP server, not more, and `..` or a
//! symlink must not turn a dependency solve into a read elsewhere.
//!
//! There is no discovery step (`sysand-index-config.json` is never read:
//! the root *is* `index_root`, and a directory has no `api_root`) and no
//! authentication. Blocking `std::fs` calls inside the async trait
//! methods are accepted: every read is a small JSON document or an
//! archive header, the same cost the synchronous
//! [`super::local_directory::LocalDirectoryEnvironment`] pays.

use std::{
    collections::{BTreeMap, HashMap},
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
        local_kpar::{KparInnerPath, LocalKParError, LocalKParProject, LocalKParProjectRaw},
        utils::{FsIoError, wrapfs},
    },
    purl::PKG_SYSAND_PREFIX,
};

#[derive(Error, Debug)]
pub enum LocalIndexError {
    /// The directory exists but holds neither shape.
    #[error(
        "`{path}` is neither a Sysand index nor a folder of KPARs;\n\
        {SP:>8} a Sysand index holds `{INDEX_FILE_NAME}`, a folder of KPARs holds `*.kpar` files",
        SP = ' '
    )]
    NeitherIndexNorFolder { path: Utf8PathBuf },
    #[error("index `{path}` is not a directory")]
    NotADirectory { path: Utf8PathBuf },
    #[error("`{entry}` in `{file}` escapes the index at `{root}`")]
    EscapesIndex {
        entry: Utf8PathBuf,
        file: Utf8PathBuf,
        root: Utf8PathBuf,
    },
    /// An archive whose `.project.json` has no `publisher`, named by its
    /// location, with no `model.toml` remedy (a KPAR cannot carry one).
    /// A folder of KPARs is indexed by `pkg:sysand/<publisher>/<name>`,
    /// so an archive without a publisher has no IRI to be listed under.
    #[error(
        "`{archive}` has a `.project.json` with no `publisher`;\n\
        {SP:>8} a folder index names each archive `{PKG_SYSAND_PREFIX}<publisher>/<name>`, \
        so add one and rebuild the archive",
        SP = ' '
    )]
    NoPublisher { archive: Utf8PathBuf },
    #[error("`{archive}` has no `.project.json`")]
    MissingInfo { archive: Utf8PathBuf },
    #[error("`{archive}` declares non-semver version `{value}`: {source}")]
    NonSemverArchiveVersion {
        archive: Utf8PathBuf,
        value: String,
        #[source]
        source: semver::Error,
    },
    #[error("`{first}` and `{second}` in `{root}` both hold `{iri}` `{version}`")]
    DuplicateArchive {
        first: Utf8PathBuf,
        second: Utf8PathBuf,
        root: Utf8PathBuf,
        iri: String,
        version: String,
    },
    #[error("failed to parse JSON from `{file}`: {source}")]
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

/// One `versions.json` entry after validation, or one archive of a
/// folder: what a candidate needs to be enumerated and then opened.
#[derive(Debug, Clone)]
struct Candidate {
    version: Version,
    status: VersionStatus,
    /// Structured: the digest the entry advertises, verified against the
    /// archive when it is opened. Folder: `None`, the archive is its own
    /// authority.
    expected: Option<KparMeta>,
    /// Folder: the archive itself. Structured: `None`, the path follows
    /// from the layout.
    archive: Option<Utf8PathBuf>,
}

/// The versions of one identity, newest first.
type Candidates = Arc<[Candidate]>;

#[derive(Debug)]
enum Shape {
    Structured,
    /// The synthesized `versions.json` view of a folder: IRI to its
    /// archives, newest first; built once per environment lifetime.
    Folder(Arc<BTreeMap<String, Candidates>>),
}

/// A structured mirror or a bare folder of archives, read from `root`.
#[derive(Debug)]
pub struct LocalIndexEnvironment {
    /// The spelling the user gave, for messages.
    display: Utf8PathBuf,
    /// Absolute, as spelled (symlinks intact) -- what paths are built from.
    root: Utf8PathBuf,
    /// Canonical root plus detected shape; populated on first access.
    detected: Mutex<Option<Arc<(Utf8PathBuf, Shape)>>>,
    /// Structured shape: validated `versions.json` documents by IRI.
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
            detected: Mutex::new(None),
            versions_cache: Mutex::new(HashMap::new()),
        }
    }

    /// The directory as the user spelled it.
    pub fn display_path(&self) -> &Utf8Path {
        &self.display
    }

    fn detect(&self) -> Result<Arc<(Utf8PathBuf, Shape)>, LocalIndexError> {
        let mut guard = self
            .detected
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
        let canonical_root = wrapfs::canonicalize(&self.root)?;
        let shape = if wrapfs::is_file(self.root.join(INDEX_FILE_NAME))? {
            Shape::Structured
        } else {
            let archives = self.enumerate_archives(&canonical_root)?;
            if archives.is_empty() {
                return Err(LocalIndexError::NeitherIndexNorFolder {
                    path: self.display.clone(),
                });
            }
            Shape::Folder(Arc::new(self.tabulate_folder(archives)?))
        };
        let found = Arc::new((canonical_root, shape));
        *guard = Some(Arc::clone(&found));
        Ok(found)
    }

    /// The `*.kpar` files directly under the root (not recursive), by
    /// name, each canonicalized and checked to stay under the root.
    fn enumerate_archives(
        &self,
        canonical_root: &Utf8Path,
    ) -> Result<Vec<Utf8PathBuf>, LocalIndexError> {
        let mut names: Vec<Utf8PathBuf> = Vec::new();
        for entry in wrapfs::read_dir(&self.root)? {
            let entry = entry.map_err(|e| Box::new(FsIoError::ReadDir(self.root.clone(), e)))?;
            let path = entry.path();
            let is_kpar = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("kpar"));
            if is_kpar && wrapfs::is_file(path)? {
                names.push(path.to_path_buf());
            }
        }
        names.sort();
        names
            .into_iter()
            .map(|path| self.contained(canonical_root, &path, &self.root))
            .collect()
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

    /// Open each archive's `.project.json` and build the per-IRI table
    /// (one pass over the archives; measured in `local_index_tests`).
    fn tabulate_folder(
        &self,
        archives: Vec<Utf8PathBuf>,
    ) -> Result<BTreeMap<String, Candidates>, LocalIndexError> {
        let mut table: BTreeMap<String, Vec<(Candidate, Utf8PathBuf)>> = BTreeMap::new();
        for archive in archives {
            let raw = LocalKParProjectRaw::new_guess_root(&archive)?;
            let Some(info) = raw.get_info()? else {
                return Err(LocalIndexError::MissingInfo { archive });
            };
            let Some(publisher) = info.publisher else {
                return Err(LocalIndexError::NoPublisher { archive });
            };
            let version = Version::parse(&info.version).map_err(|source| {
                LocalIndexError::NonSemverArchiveVersion {
                    archive: archive.clone(),
                    value: info.version.clone(),
                    source,
                }
            })?;
            let iri = format!("{PKG_SYSAND_PREFIX}{publisher}/{}", info.name);
            let held = table.entry(iri.clone()).or_default();
            if let Some((_, first)) = held.iter().find(|(c, _)| c.version == version) {
                return Err(LocalIndexError::DuplicateArchive {
                    first: first.clone(),
                    second: archive,
                    root: self.display.clone(),
                    iri,
                    version: version.to_string(),
                });
            }
            held.push((
                Candidate {
                    version,
                    status: VersionStatus::Available,
                    expected: None,
                    archive: Some(archive.clone()),
                },
                archive,
            ));
        }
        Ok(table
            .into_iter()
            .map(|(iri, mut held)| {
                held.sort_by(|(a, _), (b, _)| b.version.cmp(&a.version));
                (iri, held.into_iter().map(|(c, _)| c).collect())
            })
            .collect())
    }

    /// `<root>/<publisher>/<name>` (or `<root>/_iri/<sha256>`), the
    /// structured layout's project directory.
    fn project_dir(&self, iri: &str) -> Result<Utf8PathBuf, LocalIndexError> {
        let [first, second] = parse_iri(iri)?.get_path_segments();
        Ok(self.root.join(first).join(second))
    }

    /// Structured shape: the validated `versions.json` of `iri`, or `None`
    /// when the file is absent (the project is not in this index -- the
    /// HTTP client's 404 rule).
    fn structured_versions(
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
                expected: Some(KparMeta {
                    size_bytes: advertised.kpar_size,
                    sha256_hex: advertised.kpar_digest.as_hex().to_owned(),
                }),
                archive: None,
            })
            .collect();
        self.versions_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(iri.to_owned(), Arc::clone(&validated));
        Ok(Some(validated))
    }

    fn candidates(&self, iri: &str) -> Result<Option<Candidates>, LocalIndexError> {
        let detected = self.detect()?;
        let (canonical_root, shape) = &*detected;
        match shape {
            Shape::Structured => self.structured_versions(canonical_root, iri),
            Shape::Folder(table) => Ok(table.get(iri).cloned()),
        }
    }
}

type ResultStream<T> = futures::stream::Iter<std::vec::IntoIter<Result<T, LocalIndexError>>>;

impl ReadEnvironmentAsync for LocalIndexEnvironment {
    type ReadError = LocalIndexError;

    type UriStream = ResultStream<String>;

    async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
        let detected = self.detect()?;
        let iris: Vec<Result<String, LocalIndexError>> = match &detected.1 {
            Shape::Structured => {
                let file = self.root.join(INDEX_FILE_NAME);
                let text = wrapfs::read_to_string(&file)?;
                let index: IndexJson = serde_json::from_str(&text)
                    .map_err(|source| LocalIndexError::Json { file, source })?;
                index
                    .projects
                    .into_iter()
                    .filter(|p| p.status == ProjectStatus::Available)
                    .map(|p| Ok(p.iri))
                    .collect()
            }
            Shape::Folder(table) => table.keys().cloned().map(Ok).collect(),
        };
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
        // Retired entries are excluded from new resolutions, as the HTTP
        // client excludes them (`design/index-protocol.md` §12).
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
        let detected = self.detect()?;
        let (canonical_root, shape) = &*detected;
        let versions_file = match shape {
            Shape::Structured => self.project_dir(iri)?.join(VERSIONS_FILE_NAME),
            Shape::Folder(_) => self.root.clone(),
        };
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
        // A non-semver request names nothing an index lists; the parse
        // error adds nothing to "not listed".
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
        let archive = if let Some(archive) = &candidate.archive {
            archive.clone()
        } else {
            let named = self
                .project_dir(iri)?
                .join(candidate.version.to_string())
                .join(KPAR_FILE_NAME);
            self.contained(canonical_root, &named, &versions_file)?
        };
        // An index usage's identity is its PURL; that the archive
        // declares it is checked by the lock, as for the HTTP arm
        // (`commands::lock::check_typed_usages`).
        let project = LocalKParProject::new_access(archive, KparInnerPath::Root, None);
        let project = match &candidate.expected {
            Some(expected) => project.expecting(expected.clone()),
            None => project,
        };
        Ok(project.to_async())
    }

    async fn has_version_verified_async<S: AsRef<str>, V: AsRef<str>>(
        &self,
        _uri: S,
        _version: V,
        _checksum: &crate::env::ProjectChecksum,
    ) -> Result<crate::env::ProjectChecksumResult, Self::ReadError> {
        // As the HTTP index: an index is a resolution source, never the
        // environment a lockfile is verified against.
        panic!("has_version_verified_async is not meaningful for an index")
    }
}

#[cfg(test)]
#[path = "./local_index_tests.rs"]
mod tests;
