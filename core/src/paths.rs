// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Validation, normalization and relativization of file paths.

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use thiserror::Error;
use typed_path::{Utf8UnixComponent, Utf8UnixPath, Utf8UnixPathBuf, Utf8WindowsPath};

#[cfg(feature = "filesystem")]
use crate::project::utils::{ToPathBuf as _, wrapfs};

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum RelativeUnixPathError {
    #[error("path `{path}` is absolute")]
    Absolute { path: String },
    #[error(
        "path `{path}` contains a backslash `\\`;\n\
        backslash is not allowed in paths to preserve consistent\n\
        interpretation across different operating systems;\n\
        backslash could be present because the path is Windows path"
    )]
    ContainsBackslash { path: String },
    #[error(
        "path `{path}` contains two consecutive\n\
        directory separators `/`, which is forbidden"
    )]
    ContainsDoubleSlash { path: String },
    /// Contains `.` component
    #[error("path `{path}` contains a forbidden relative component `.`")]
    ContainsCurrent { path: String },
    /// Contains `..` component
    #[error("path `{path}` contains a forbidden relative component `..`")]
    ContainsParent { path: String },
    /// Contains a `..` component after a normal one
    #[error("path `{path}` contains a `..` component that is not one of the leading components")]
    ContainsNonLeadingParent { path: String },
    #[error("path `{path:?}` contains a NULL character")]
    ContainsNull { path: String },
    #[error("path `{path}` ends with a separator `/`, which is forbidden")]
    TrailingSlash { path: String },
    #[error(
        "path `{path}` is a directory path\n\
        (ends with a separator `/`), but a file path was expected"
    )]
    DirectoryPath { path: String },
}

/// A path that [`normalize_relative`] rejects.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum NormalizeError {
    #[error("path `{path}` is absolute")]
    Absolute { path: String },
    #[error(
        "path `{path}` contains a backslash `\\`;\n\
        backslash is not allowed in paths to preserve consistent\n\
        interpretation across different operating systems;\n\
        backslash could be present because the path is Windows path"
    )]
    ContainsBackslash { path: String },
    #[error("path `{path:?}` contains a NULL character")]
    ContainsNull { path: String },
    /// The path points above the root it is relative to.
    #[error("path `{path}` points above the root it is relative to")]
    EscapesRoot { path: String },
    /// The path is the root it is relative to.
    #[error("path `{path}` is the root it is relative to, not a path inside it")]
    IsRoot { path: String },
}

// TODO: use newtypes for different kinds of paths
//
// Some rules should not be enforced at parse time; they are more like
// normalization:
// - no `.` components
// - no trailing slashes - this must not fail parsing for dirs
//
// Desirable variants of relative paths:
// - file subpath: no relative components, no trailing slash:
//   - meta.index
//   - meta.checksum
// - file path: no trailing slash:
//   - some OverrideSource paths
// - directory path:
//   - some OverrideSource paths
// - directory subpath:
//   - env.toml paths

/// What the path is intended to be
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelativePathKind {
    /// No relative components, no trailing slash
    SubFile,
    /// No trailing slash
    File,
    /// No relative components, may have trailing slash
    SubDirectory,
    /// No restrictions
    Directory,
}

impl RelativePathKind {
    fn is_file(self) -> bool {
        match self {
            Self::SubFile | Self::File => true,
            Self::SubDirectory | Self::Directory => false,
        }
    }

    fn allow_relative_components(self) -> bool {
        match self {
            Self::SubFile | Self::SubDirectory => false,
            Self::File | Self::Directory => true,
        }
    }
}

/// Whether `path` is absolute on Unix or on Windows. A Windows drive
/// letter counts, so `C:/main.sysml` is absolute.
pub fn is_absolute_on_any_os(path: &str) -> bool {
    path.starts_with('/') || Utf8WindowsPath::new(path).is_absolute()
}

// TODO: use this in config validation

/// Parse a relative file path that uses `/` as separator.
/// If `allow_relative_components == true`, components `.` and `..` are allowed.
/// Trailing slash is not allowed.
pub fn parse_relative_unix_path(
    path: &str,
    kind: RelativePathKind,
) -> Result<&Utf8UnixPath, RelativeUnixPathError> {
    parse_unix_path(path, kind, false)
}

/// [`parse_relative_unix_path`] for a path that may also be absolute.
/// An absolute path is held to the same rules past its root.
pub fn parse_unix_path_or_absolute(
    path: &str,
    kind: RelativePathKind,
) -> Result<&Utf8UnixPath, RelativeUnixPathError> {
    parse_unix_path(path, kind, true)
}

/// Parse a `dir` or `kpar-path` as written: a path
/// [`parse_unix_path_or_absolute`] accepts, with `.` and a trailing slash rejected
/// and `..` allowed only as the leading components of a relative path.
/// Unlike [`normalize_relative`] the path may leave the directory it is
/// relative to. An absolute path is held to the same rules past its root.
pub fn parse_local_path(path: &str) -> Result<&Utf8UnixPath, RelativeUnixPathError> {
    if path == "." {
        return Err(RelativeUnixPathError::ContainsCurrent {
            path: path.to_owned(),
        });
    }
    parse_unix_path_or_absolute(path, RelativePathKind::Directory)?;
    if path.ends_with('/') {
        return Err(RelativeUnixPathError::TrailingSlash {
            path: path.to_owned(),
        });
    }
    let mut leading = !is_absolute_on_any_os(path);
    for part in path.split('/').filter(|part| !part.is_empty()) {
        match part {
            "." => {
                return Err(RelativeUnixPathError::ContainsCurrent {
                    path: path.to_owned(),
                });
            }
            ".." if !leading => {
                return Err(RelativeUnixPathError::ContainsNonLeadingParent {
                    path: path.to_owned(),
                });
            }
            ".." => {}
            _ => leading = false,
        }
    }
    Ok(Utf8UnixPath::new(path))
}

fn parse_unix_path(
    path: &str,
    kind: RelativePathKind,
    allow_absolute: bool,
) -> Result<&Utf8UnixPath, RelativeUnixPathError> {
    // Path "." is always allowed for directories
    if !kind.is_file() && path == "." {
        return Ok(Utf8UnixPath::new(path));
    }

    let body = if is_absolute_on_any_os(path) {
        if !allow_absolute {
            return Err(RelativeUnixPathError::Absolute {
                path: path.to_owned(),
            });
        }
        // Past the root of a Unix path; a Windows drive is an ordinary
        // first component here.
        path.strip_prefix('/').unwrap_or(path)
    } else {
        path
    };

    match first_nul_or_backslash(body) {
        Some(b'\0') => {
            return Err(RelativeUnixPathError::ContainsNull {
                path: path.to_owned(),
            });
        }
        Some(_) => {
            return Err(RelativeUnixPathError::ContainsBackslash {
                path: path.to_owned(),
            });
        }
        None => {}
    }
    let stripped = if let Some(s) = body.strip_suffix('/') {
        if kind.is_file() {
            return Err(RelativeUnixPathError::DirectoryPath {
                path: path.to_owned(),
            });
        } else {
            s
        }
    } else {
        body
    };

    // Manually split into components, `Utf8UnixPath::components()` does
    // some normalization, which is undesirable here
    for c in stripped.split('/') {
        if c.is_empty() {
            // trailing slash is already stripped, and the path is
            // not absolute, so the only way for it to contain an empty
            // component is to have two consecutive slashes
            return Err(RelativeUnixPathError::ContainsDoubleSlash {
                path: path.to_owned(),
            });
        }
        if !kind.allow_relative_components() {
            if c == "." {
                return Err(RelativeUnixPathError::ContainsCurrent {
                    path: path.to_owned(),
                });
            } else if c == ".." {
                return Err(RelativeUnixPathError::ContainsParent {
                    path: path.to_owned(),
                });
            }
        }
    }

    Ok(Utf8UnixPath::new(path))
}

/// Lexically normalize a `/`-separated relative path: a leading `./` and
/// inner `.`/`..` components are resolved, repeated slashes are collapsed
/// and a trailing slash is dropped. Works on the path text alone.
///
/// Unlike [`parse_relative_unix_path`] it resolves `.` and `..` components
/// rather than rejecting them, and complains only if the result leaves the
/// root or *is* the root. Absolute paths (Unix and Windows alike),
/// backslashes and NUL characters are rejected exactly as there.
pub fn normalize_relative(path: &str) -> Result<Utf8UnixPathBuf, NormalizeError> {
    if is_absolute_on_any_os(path) {
        return Err(NormalizeError::Absolute {
            path: path.to_owned(),
        });
    }
    match first_nul_or_backslash(path) {
        Some(b'\0') => {
            return Err(NormalizeError::ContainsNull {
                path: path.to_owned(),
            });
        }
        Some(_) => {
            return Err(NormalizeError::ContainsBackslash {
                path: path.to_owned(),
            });
        }
        None => {}
    }

    let segments = resolve_dot_components(path);
    // `..` can only be left over at the start, where it has nothing to cancel
    if segments.first() == Some(&"..") {
        return Err(NormalizeError::EscapesRoot {
            path: path.to_owned(),
        });
    }
    if segments.is_empty() {
        return Err(NormalizeError::IsRoot {
            path: path.to_owned(),
        });
    }

    Ok(Utf8UnixPathBuf::from(segments.join("/")))
}

/// The first NUL or backslash byte of `path`, if it has one.
fn first_nul_or_backslash(path: &str) -> Option<u8> {
    path.bytes().find(|c| matches!(c, b'\0' | b'\\'))
}

/// Lexically resolve the components of a `/`-separated path: empty and `.`
/// components are dropped, and a `..` cancels the component before it unless
/// that is a `..` too, so `..` is left only at the start.
pub(crate) fn resolve_dot_components(path: &str) -> Vec<&str> {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." if segments.last().is_some_and(|last| *last != "..") => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    segments
}

/// `base` with `relative` appended component by component: `.` is
/// skipped and `..` drops the last component of the result. Works on the
/// path text alone.
pub fn join_lexical(base: &Utf8Path, relative: &Utf8Path) -> Utf8PathBuf {
    let mut joined = base.to_path_buf();
    for component in relative.components() {
        match component {
            Utf8Component::ParentDir => {
                joined.pop();
            }
            Utf8Component::CurDir => {}
            other => joined.push(other.as_str()),
        }
    }
    joined
}

#[derive(Debug, Error, Clone)]
pub enum RelativizePathError {
    #[error("unable to relativize path `{path}` with respect to `{root}`")]
    NoCommonPrefix {
        path: Box<Utf8Path>,
        root: Box<Utf8Path>,
    },
}

// Note: `components()` ignores non-leading `CurDir` so paths like `/a/./b`
// are reported as canonical.
fn contains_non_canonical_components(path: &Utf8Path) -> bool {
    path.components()
        .any(|c| matches!(c, Utf8Component::CurDir | Utf8Component::ParentDir))
}

/// Computes the relative path from `root` to `path`.
///
/// Both `path` and `root` must be absolute and structurally canonical:
///
/// - They must be absolute paths.
/// - They must not contain `.` (`CurDir`) components.
/// - They must not contain `..` (`ParentDir`) components.
/// - They must share the same path prefix (e.g., drive letter on Windows).
///
/// # Panics
///
/// Panics if any of these preconditions is violated. They are invariants
/// for the caller to uphold; user input is checked elsewhere.
///
/// This function manipulates the path text alone: it leaves the filesystem
/// and symlinks untouched. Callers are expected
/// to pass paths that have been canonicalized beforehand (e.g., via
/// [`crate::project::utils::wrapfs::canonicalize`] or equivalent).
///
/// # Returns
///
/// - `Ok(relative_path)` if a relative path from `root` to `path` can be computed.
/// - `Err(RelativizePathError)` if the paths do not share a common prefix (should
///   be relevant only on Windows).
///
/// If `path` and `root` are identical, a `Utf8PathBuf` with a single `.` component
/// is returned.
///
/// # Examples
///
/// ```rust
/// # use camino::Utf8Path;
/// # use sysand_core::paths::relativize_path;
/// let path = if cfg!(windows) {
///     Utf8Path::new(r"C:\a\b\c")
/// } else {
///     Utf8Path::new("/a/b/c")
/// };
/// let root = if cfg!(windows) {
///     Utf8Path::new(r"C:\a\b")
/// } else {
///     Utf8Path::new("/a/b")
/// };
///
/// let relative = relativize_path(path, root).unwrap();
/// assert_eq!(relative, "c");
/// ```
///
/// ```rust
/// # use camino::Utf8Path;
/// # use sysand_core::paths::relativize_path;
/// let path = if cfg!(windows) {
///     Utf8Path::new(r"C:\a\b")
/// } else {
///     Utf8Path::new("/a/b")
/// };
/// let root = if cfg!(windows) {
///     Utf8Path::new(r"C:\a\b\c")
/// } else {
///     Utf8Path::new("/a/b/c")
/// };
///
/// let relative = relativize_path(path, root).unwrap();
/// assert_eq!(relative, "..");
/// ```
pub fn relativize_path<P: AsRef<Utf8Path>, R: AsRef<Utf8Path>>(
    path: P,
    root: R,
) -> Result<Utf8UnixPathBuf, RelativizePathError> {
    let path = path.as_ref();
    let root = root.as_ref();

    // Internal invariants: callers canonicalize user input before getting here.
    assert!(path.is_absolute(), "path `{path}` is not absolute");
    assert!(root.is_absolute(), "root `{root}` is not absolute");
    assert!(
        !contains_non_canonical_components(path),
        "path `{path}` contains a `.` or `..` component"
    );
    assert!(
        !contains_non_canonical_components(root),
        "root `{root}` contains a `.` or `..` component"
    );

    let mut path_iter = path.components().peekable();
    let mut root_iter = root.components().peekable();

    // Both paths are absolute, so prefixes can only differ on Windows in the cases when:
    // - paths point to different drives
    // - path kinds differ (regular vs UNC vs DOS device paths)
    // Failure should not be possible on Unix-like systems, since absolute
    // paths all start with the common prefix `/`.
    match (path_iter.peek(), root_iter.peek()) {
        (Some(p0), Some(r0)) if p0 == r0 => {
            path_iter.next();
            root_iter.next();
        }
        _ => {
            return Err(RelativizePathError::NoCommonPrefix {
                path: path.into(),
                root: root.into(),
            });
        }
    }

    while let (Some(p), Some(r)) = (path_iter.peek(), root_iter.peek()) {
        if p == r {
            path_iter.next();
            root_iter.next();
        } else {
            break;
        }
    }

    let mut result = Utf8UnixPathBuf::new();

    for r in root_iter {
        if let Utf8Component::Normal(_) = r {
            result.push("..");
        }
    }

    // TODO: it's probably always expected that we will use Unix paths
    for p in path_iter {
        result.push(p.as_str());
    }

    if result.as_str().is_empty() {
        result.push(".");
    }

    Ok(result.into_string().into())
}

/// Whether a path made relative to the workspace root climbs out of it.
pub fn is_above_root(relative: &Utf8UnixPath) -> bool {
    relative
        .components()
        .any(|c| c == Utf8UnixComponent::ParentDir)
}

/// A path that could not be made absolute or canonicalized.
#[cfg(feature = "filesystem")]
#[derive(Debug, Error)]
#[error("failed to resolve path `{path}`")]
pub struct ResolvePathError {
    pub path: Utf8PathBuf,
    #[source]
    pub source: std::io::Error,
}

/// Tries to canonicalize the (longest possible) prefix of a path.
/// Useful if you have /path/to/file/that/does/not/exist
/// but where some prefix, say, /path/to/file can be canonicalized.
///
/// The part that does not exist is appended as written, including any `..`.
/// Any other canonicalization failure, such as a permission error or a path
/// inside a regular file, is returned.
#[cfg(feature = "filesystem")]
pub(crate) fn canonicalize_prefix<P: AsRef<Utf8Path>>(
    path: P,
) -> Result<Utf8PathBuf, ResolvePathError> {
    let mut relative_part = Utf8PathBuf::new();
    let mut absolute_part = path.to_path_buf();

    loop {
        match wrapfs::canonicalize_raw(&absolute_part) {
            Ok(canonical_absolute) => {
                absolute_part = canonical_absolute;
                break;
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ResolvePathError {
                    path: absolute_part,
                    source,
                });
            }
        }

        match (absolute_part.parent(), absolute_part.file_name()) {
            (Some(absolute_part_parent), Some(absolute_part_file)) => {
                relative_part = Utf8Path::new(absolute_part_file).join(relative_part);
                absolute_part = absolute_part_parent.to_path_buf();
            }
            _ => {
                break;
            }
        }
    }

    absolute_part.push(relative_part);
    Ok(absolute_part)
}

/// `path` relative to `relative_to`, or `None` if `path` is outside it.
#[cfg(feature = "filesystem")]
pub(crate) fn relativize_path_in<P: AsRef<Utf8Path>, Q: AsRef<Utf8Path>>(
    path: P,
    relative_to: Q,
) -> Result<Option<Utf8UnixPathBuf>, ResolvePathError> {
    let path = if path.as_ref().is_absolute() {
        canonicalize_prefix(path)?
    } else {
        let absolute = camino::absolute_utf8(path.as_ref()).map_err(|source| ResolvePathError {
            path: path.as_ref().to_owned(),
            source,
        })?;
        canonicalize_prefix(absolute)?
    };

    let root = canonicalize_prefix(relative_to)?;
    // `..` can survive in the part of a path that does not exist, and
    // `relativize_path` requires none
    if contains_non_canonical_components(&path) || contains_non_canonical_components(&root) {
        return Ok(None);
    }
    // `NoCommonPrefix` is the only error and means `path` is outside `root`
    // ast-grep-ignore: dot-ok
    let Some(relative) = relativize_path(path, root).ok() else {
        return Ok(None);
    };
    if is_above_root(&relative) {
        return Ok(None);
    }
    // `relativize_path` names the root itself `.`; here it is the empty path
    if relative.as_str() == "." {
        return Ok(Some(Utf8UnixPathBuf::new()));
    }
    Ok(Some(relative))
}

#[cfg(test)]
#[path = "./paths_tests.rs"]
mod tests;
