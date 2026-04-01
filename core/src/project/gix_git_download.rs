use std::{
    num::{NonZero, NonZeroU32},
    sync::atomic::AtomicBool,
};

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;
use gix::{
    prepare_clone,
    progress::{self, Discard},
    remote::{
        Direction,
        fetch::{self, Shallow},
    },
};
use serde::Deserialize;
use thiserror::Error;
use walkdir::WalkDir;

use crate::{
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectRead,
        local_src::{LocalSrcError, LocalSrcProject, PathError},
        utils::{FileWithLifetime, RelativizePathError, ToPathBuf},
    },
};

use super::utils::{FsIoError, ProjectDeserializationError, ProjectSerializationError, wrapfs};

// easyfind2
//
// Uses of GixDownloadedProject:
// - resolver candidate project (i.e. legacy IRI) (unknown if remote exists/is a git repo). Known info:
//   - URL
//   Known after constructor: nothing new (should not clone in constructor)
//   Needed functionality:
//   - clone repo
//   - impl ProjectRead -- so it's immutable when the actual repo clone happens and so cannot fill its fields
//   - get sufficient info for lockfile: rev
//   Need to carry:
//   - repo URL (constructor does not clone)
//   - path in repo
//   - exact rev
// - sync. Known info:
//   - URL,
//   - path in repo,
//   - rev
//   Known after constructor (should clone in constructor, and therefore
//   carry around info/meta):
//   - (optional) publisher/name
//   Needed functionality:
//   - clone repo
//   - impl ProjectRead
//   Need to carry:
//   - info/meta (info already read in lockfile, so might as well read meta)
//   - path in repo
//   - exact rev
// - new style usage. Known info:
//   - URL
//   - publisher
//   - name
//   - rev (optional, if none then assume latest)
//   Known after constructor (should clone in constructor and therefore carry around info/meta):
//   - path in repo
//   - exact rev
//   - (optional) publisher/name
//   Needed functionality:
//   - clone repo
//   - impl ProjectRead
//   Need to carry:
//   - url (for lockfile)
//   - info/meta (info already read in lockfile, so might as well read meta)
//   - path in repo
//   - exact rev
//
// Lockfile generation needs:
// - project info/meta
// - exact rev
// - path in repo (if any)

#[derive(Debug)]
pub struct GixDownloadedProject {
    pub url: gix::Url,
    /// Before cloning: git rev to clone.
    /// After cloning: actual git rev, will match requested if given,
    /// otherwise the latest rev of the default branch.
    // rev: Option<String>,
    /// path within the cloned repo where project resides
    // path: Option<String>,
    tmp_dir: camino_tempfile::Utf8TempDir,
    inner: LocalSrcProject,
}

#[derive(Debug)]
pub struct GixDownloadedProjectExact {
    url: String,
    /// Git rev of the project. Will match the given one or be the
    /// latest on the default branch if none is given
    rev: String,
    /// Path within the cloned repo where project resides.
    /// If None, project is at root.
    path: Option<Utf8PathBuf>,
    tmp_dir: camino_tempfile::Utf8TempDir,
    inner: LocalSrcProject,
}

#[derive(Error, Debug)]
pub enum GixDownloadedError {
    #[error("git clone from `{0}` failed: {1}")]
    Clone(String, Box<gix::clone::Error>),
    // #[error("git bare repo init at `{0}` failed: {1}")]
    // Init(String, Box<gix::init::Error>),
    // #[error("git remote `{0}` init failed: {1}")]
    // RemoteInit(String, Box<gix::remote::init::Error>),
    #[error("failed to parse git URL `{0}`: {1}")]
    UrlParse(Box<str>, Box<gix::url::parse::Error>),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Serialize(#[from] ProjectSerializationError),
    #[error("git fetch from `{0}` failed: {1}")]
    Fetch(String, Box<gix::clone::fetch::Error>),
    #[error("git checkout in temporary directory `{0}` failed: {1}")]
    Checkout(Utf8PathBuf, Box<gix::clone::checkout::main_worktree::Error>),
    #[error(
        "cannot construct a relative path from the workspace/project
        directory to one of its dependencies' directory:\n\
        {0}"
    )]
    ImpossibleRelativePath(#[from] RelativizePathError),
    #[error(
        "project with publisher `{publisher}` and name `{name}`\n\
        not found in `{repo_url}` at rev {rev}"
    )]
    ProjectNotFound {
        repo_url: Box<str>,
        rev: Box<str>,
        publisher: Box<str>,
        name: Box<str>,
    },
    #[error("{0}")]
    Other(String),
}

impl From<FsIoError> for GixDownloadedError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl From<LocalSrcError> for GixDownloadedError {
    fn from(value: LocalSrcError) -> Self {
        match value {
            LocalSrcError::Deserialize(error) => Self::Deserialize(error),
            LocalSrcError::Path(error) => Self::Path(error),
            LocalSrcError::AlreadyExists(msg) => {
                GixDownloadedError::Other(format!("unexpected internal error: {}", msg))
            }
            LocalSrcError::Io(e) => Self::Io(e),
            LocalSrcError::Serialize(error) => Self::Serialize(error),
            LocalSrcError::ImpossibleRelativePath(err) => Self::ImpossibleRelativePath(err),
            LocalSrcError::MissingMeta => GixDownloadedError::Other(
                "project is missing metadata file `.meta.json`".to_string(),
            ),
        }
    }
}

impl GixDownloadedProjectExact {
    /// Immediately clone the repo and try to find the project publisher/name
    pub fn new_download_find<S: AsRef<str>>(
        url: S,
        rev: Option<String>,
        publisher: impl AsRef<str>,
        name: impl AsRef<str>,
    ) -> Result<GixDownloadedProjectExact, GixDownloadedError> {
        let url = url.as_ref();
        let publisher = publisher.as_ref();
        let name = name.as_ref();
        let tmp_dir = camino_tempfile::tempdir().map_err(FsIoError::MkTempDir)?;
        let repo = download_repo_to_temp(&tmp_dir, url, rev.as_deref())?;

        let rev = rev.unwrap_or_else(|| repo.head_commit().unwrap().id().to_string());

        // TODO: find specified project in repo and convert to path
        // TODO: Since gix provides a way to iterate over non-checked-out files,
        // checkout may not be necessary.

        // Check every `.project.json` file
        for entry in WalkDir::new(tmp_dir.path())
            .into_iter()
            .filter_entry(|entry| entry.file_name() == ".git")
        {
            match entry {
                Ok(entry) => {
                    if !entry.file_type().is_file() || entry.path().ends_with(".project.json") {
                        continue;
                    }
                    let Some(path) = entry.path().to_str() else {
                        log::debug!(
                            "ignoring path `{}` as it contains invalid Unicode",
                            entry.path().display()
                        );
                        continue;
                    };
                    let info: InterchangeProjectInfoRaw =
                        match serde_json::from_reader(wrapfs::File::open(path)?) {
                            Ok(info) => info,
                            Err(e) => {
                                log::debug!(
                                    "ignoring file `{}` due to error: {e}",
                                    entry.path().display()
                                );
                                continue;
                            }
                        };

                    if info.publisher.as_deref() == Some(publisher) && info.name == name {
                        // FOUND
                        // let mut canonical_temp = wrapfs::canonicalize(tmp_dir.path())?;
                        // Append path inside the repo, as it will be cloned to the temp dir
                        let downloaded_project = LocalSrcProject {
                            nominal_path: None,
                            project_path: entry.path().parent().unwrap().to_str().unwrap().into(),
                        };
                        let path_in_repo = downloaded_project
                            .project_path
                            .strip_prefix(tmp_dir.path())
                            .unwrap();
                        return Ok(GixDownloadedProjectExact {
                            url: url.to_owned(),
                            rev,
                            path: if path_in_repo.as_str().is_empty() {
                                None
                            } else {
                                Some(path_in_repo.to_owned())
                            },
                            inner: downloaded_project,
                            tmp_dir,
                        });
                    }
                }
                Err(e) => {
                    log::debug!("skipping path due to error: {e}");
                }
            }
        }

        Err(GixDownloadedError::ProjectNotFound {
            repo_url: url.into(),
            rev: rev.into(),
            publisher: publisher.into(),
            name: name.into(),
        })
    }

    /// `path` must be relative path inside repo
    pub fn new_download<S: AsRef<str>>(
        url: S,
        rev: String,
        path: Option<String>,
    ) -> Result<GixDownloadedProjectExact, GixDownloadedError> {
        let url = url.as_ref();
        let tmp_dir = camino_tempfile::tempdir().map_err(FsIoError::MkTempDir)?;
        let _repo = download_repo_to_temp(&tmp_dir, url, Some(&rev))?;

        let downloaded_project = LocalSrcProject {
            nominal_path: None,
            project_path: if let Some(p) = &path {
                tmp_dir.path().join(p)
            } else {
                tmp_dir.path().into()
            },
        };
        Ok(GixDownloadedProjectExact {
            url: url.to_owned(),
            rev,
            path: path.map(Into::into),
            inner: downloaded_project,
            tmp_dir,
        })
    }
}

impl GixDownloadedProject {
    pub fn new<S: AsRef<str>>(
        url: S,
        // rev: Option<String>,
        // path: Option<String>,
    ) -> Result<GixDownloadedProject, GixDownloadedError> {
        let tmp_dir = camino_tempfile::tempdir().map_err(FsIoError::MkTempDir)?;

        let mut canonical_temp = wrapfs::canonicalize(tmp_dir.path())?;
        // Append path inside the repo, as it will be cloned to the temp dir
        // if let Some(p) = &path {
        //     canonical_temp = canonical_temp.join(p);
        // }
        let downloaded_project = LocalSrcProject {
            nominal_path: None,
            project_path: canonical_temp,
        };
        Ok(GixDownloadedProject {
            url: gix::url::parse(url.as_ref().into())
                .map_err(|e| GixDownloadedError::UrlParse(url.as_ref().into(), Box::new(e)))?,
            // rev,
            // path,
            inner: downloaded_project,
            tmp_dir,
        })
    }

    // TODO: be more efficient. Git repos should be in user-level cache
    // and updated when needed
    fn ensure_downloaded(&self) -> Result<(), GixDownloadedError> {
        if !self.tmp_dir.path().join(".git").is_dir() {
            // Try downloading only the rev we need
            // let repo = gix::init_bare(&self.tmp_dir.path()).map_err(|e| {
            //     GixDownloadedError::Init(self.tmp_dir.path().as_str().to_owned(), e.into())
            // })?;
            // let mut remote = repo
            //     .remote_at(self.url.clone())
            //     .map_err(|e| GixDownloadedError::RemoteInit(self.url.to_string(), e.into()))?;
            // //
            // // let target_commit =
            // //     gix::ObjectId::from_hex(self.rev.as_bytes()).expect("BUG: unvalidated git rev");
            // // The exact name here doesn't matter, we'll only clone a single commit anyway.
            // remote
            //     .replace_refspecs(
            //         [format!("{}:FETCH_HEAD", self.rev).as_str()],
            //         Direction::Fetch,
            //     )
            //     .unwrap();
            // // TODO: proper error reporting
            // let outcome = remote
            //     .connect(Direction::Fetch)
            //     .unwrap()
            //     .prepare_fetch(progress::Discard, Default::default())
            //     .unwrap()
            //     .with_shallow(Shallow::DepthAtRemote(NonZeroU32::new(1).unwrap()))
            //     .receive(progress::Discard, &gix::interrupt::IS_INTERRUPTED)
            //     .unwrap();
            // // TODO: check that it actually fetched what we want

            // Clone the repo, the checkout `rev` (which must be a commit SHA1/256).
            // Adapted from gitoxide `main_worktree()`:
            // https://github.com/GitoxideLabs/gitoxide/blob/v0.52.0/gix/src/clone/checkout.rs#L85

            // TODO: avoid reparsing URL
            download_repo_to_temp(&self.tmp_dir, &self.url.to_string(), None)?;
        }

        Ok(())
    }
}

/// Clone the repo, the checkout `rev` (which must be a commit SHA1/256).
/// Adapted from gitoxide `main_worktree()`:
/// https://github.com/GitoxideLabs/gitoxide/blob/v0.52.0/gix/src/clone/checkout.rs#L85
fn download_repo_to_temp(
    tmp_dir: &Utf8TempDir,
    url: &str,
    rev: Option<&str>,
) -> Result<gix::Repository, GixDownloadedError> {
    let repo = if let Some(rev) = rev {
        // Fetch all objects without checking out any files
        let (repo, _) = gix::prepare_clone(url, tmp_dir.path())
            .unwrap()
            .fetch_only(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
            .unwrap();

        // Resolve the SHA to a commit, then get its tree
        // We already checked that this is a valid SHA1/256
        let commit_id = gix::ObjectId::from_hex(rev.as_bytes()).unwrap();
        let tree_id = repo
            .find_object(commit_id)
            .unwrap()
            .into_commit()
            .tree_id()
            .unwrap()
            .detach();

        // Build an index from that specific tree
        let mut index = repo.index_from_tree(&tree_id).unwrap();

        // Use IdMapping as attribute source: workdir is empty, read attrs from ODB
        let mut opts = repo
            .checkout_options(gix_worktree::stack::state::attributes::Source::IdMapping)
            .unwrap();
        opts.destination_is_initially_empty = true;

        gix_worktree_state::checkout(
            &mut index,
            tmp_dir.path(),
            repo.objects.clone().into_arc().unwrap(),
            &gix::progress::Discard,
            &gix::progress::Discard,
            &gix::interrupt::IS_INTERRUPTED,
            opts,
        )
        .unwrap();

        index.write(Default::default()).unwrap();
        repo
    } else {
        let prepared_clone = prepare_clone(url.clone(), tmp_dir.path())
            .map_err(|e| GixDownloadedError::Clone(url.to_string(), Box::new(e)))?;

        let (mut prepare_checkout, _) = prepared_clone
            .with_shallow(Shallow::DepthAtRemote(NonZero::new(1).unwrap()))
            .fetch_then_checkout(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
            .map_err(|e| GixDownloadedError::Fetch(url.to_string(), Box::new(e)))?;
        let (repo, _) = prepare_checkout
            .main_worktree(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
            .map_err(|e| GixDownloadedError::Checkout(tmp_dir.to_path_buf(), Box::new(e)))?;

        repo
    };

    Ok(repo)
}

impl ProjectRead for GixDownloadedProject {
    type Error = GixDownloadedError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.ensure_downloaded()?;

        Ok(self.inner.get_project()?)
    }

    type SourceReader<'a>
        = FileWithLifetime<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.ensure_downloaded()?;

        Ok(FileWithLifetime::new(self.inner.read_source(path)?))
    }

    // TODO: find a less hacky way to provide the SHA here, it should be saved when
    // repo is cloned
    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        // TODO: be more efficient
        self.ensure_downloaded()?;
        let repo = gix::open(self.tmp_dir.path()).unwrap();

        let rev = repo.head_commit().unwrap().id().to_string();

        Ok(vec![Source::RemoteGit {
            remote_git: self.url.to_string(),
            rev,
            path: None,
        }])
    }
}

impl ProjectRead for GixDownloadedProjectExact {
    type Error = GixDownloadedError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        Ok(self.inner.get_project()?)
    }

    type SourceReader<'a>
        = FileWithLifetime<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        Ok(FileWithLifetime::new(self.inner.read_source(path)?))
    }

    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![Source::RemoteGit {
            remote_git: self.url.clone(),
            rev: self.rev.clone(),
            path: self.path.as_ref().map(|p| p.to_string()),
        }])
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use std::{io::Read, process::Command};

    use assert_cmd::prelude::*;
    use tempfile::tempdir;

    use crate::project::{ProjectRead, gix_git_download::GixDownloadedProject};
    //use predicates::prelude::*;

    /// Initializes a git repository at `path` with a pre-configured test user.
    #[cfg(feature = "alltests")]
    fn git_init(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        Command::new("git")
            .arg("init")
            .current_dir(path)
            .output()?
            .assert()
            .success();
        Command::new("git")
            .args(["config", "user.email", "user@sysand.org"])
            .current_dir(path)
            .output()?
            .assert()
            .success();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()?
            .assert()
            .success();
        Ok(())
    }

    #[cfg(feature = "alltests")]
    #[test]
    pub fn basic_gix_access() -> Result<(), Box<dyn std::error::Error>> {
        let repo_dir = tempdir()?;
        git_init(repo_dir.path())?;

        // TODO: Replace by commands::*::do_* when sufficiently complete, also use gix to create repo?
        std::fs::write(
            repo_dir.path().join(".project.json"),
            r#"{"name":"basic_gix_access","version":"1.2.3","usage":[]}"#,
        )?;
        Command::new("git")
            .arg("add")
            .arg(".project.json")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        std::fs::write(
            repo_dir.path().join(".meta.json"),
            r#"{"index":{},"created":"123"}"#,
        )?;
        Command::new("git")
            .arg("add")
            .arg(".meta.json")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        std::fs::write(repo_dir.path().join("test.sysml"), "package Test;")?;
        Command::new("git")
            .arg("add")
            .arg("test.sysml")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        Command::new("git")
            .args(["commit", "-m", "test_commit"])
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        Command::new("git")
            .arg("update-server-info")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        let hex_commit_sha = Command::new("git")
            .arg("rev-parse")
            .arg("HEAD")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success()
            .get_output()
            .stdout
            .to_owned();

        // NOTE: Gix does not support the "dumb" HTTP protocol

        // let free_port = port_check::free_local_port().unwrap().to_string();
        // let mut server = Command::new("uv")
        //     .arg("run")
        //     .arg("--isolated")
        //     .arg("--with")
        //     .arg("rangehttpserver")
        //     .arg("-m")
        //     .arg("RangeHTTPServer")
        //     .arg(&free_port)
        //     .current_dir(repo_dir.path().join(".git"))
        //     .spawn()?;

        // sleep(Duration::from_millis(100));

        let canonical = repo_dir.path().canonicalize()?;
        // On Windows, canonicalize() returns extended-length paths with a `\\?\`
        // prefix that gix cannot parse as a valid file URL. Strip it.
        let path = canonical.to_str().unwrap();
        let path = path.strip_prefix(r"\\?\").unwrap_or(path);
        let project = GixDownloadedProject::new(
            format!("file://{path}"),
            // Some(String::from_utf8(hex_commit_sha).unwrap()),
            // None,
        )?;

        let (Some(info), Some(meta)) = project.get_project()? else {
            panic!("expected info and meta");
        };

        assert_eq!(info.name, "basic_gix_access");
        assert_eq!(meta.created, "123");

        let mut buf = String::new();
        project
            .read_source("test.sysml")?
            .read_to_string(&mut buf)?;
        assert_eq!(buf, "package Test;");

        // server.kill()?;
        Ok(())
    }
}
