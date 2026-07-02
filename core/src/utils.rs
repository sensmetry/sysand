// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt::Write as _,
};

use digest::{array::Array, typenum};
use indexmap::IndexSet;
use sha2::{Digest, Sha256};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8WindowsPath};

use crate::project::{memory::InMemoryProject, utils::Identifier};

pub type ProvidedProjects = HashMap<Identifier, Vec<InMemoryProject>>;
pub type ProvidedIdentifiers = HashSet<Identifier>;

#[cfg(feature = "filesystem")]
pub(crate) mod scheme {
    use fluent_uri::component::Scheme;
    pub const SCHEME_FILE: &Scheme = Scheme::new_or_panic("file");
    #[cfg(feature = "networking")]
    pub const SCHEME_SSH: &Scheme = Scheme::new_or_panic("ssh");
    #[cfg(feature = "networking")]
    pub const SCHEME_GIT_SSH: &Scheme = Scheme::new_or_panic("git+ssh");
    #[cfg(feature = "networking")]
    pub const SCHEME_GIT_FILE: &Scheme = Scheme::new_or_panic("git+file");
    #[cfg(feature = "networking")]
    pub const SCHEME_GIT_HTTP: &Scheme = Scheme::new_or_panic("git+http");
    #[cfg(feature = "networking")]
    pub const SCHEME_GIT_HTTPS: &Scheme = Scheme::new_or_panic("git+https");
    pub const SCHEME_HTTP: &Scheme = Scheme::new_or_panic("http");
    pub const SCHEME_HTTPS: &Scheme = Scheme::new_or_panic("https");
}

/// Format an error, together with a chain of its `source()`.
/// Should be used in all cases where an error is turned into
/// a string and `source()` is not checked
pub fn format_err(error: impl Error) -> String {
    let mut error: &dyn Error = &error;
    let mut message = error.to_string();
    while let Some(source) = error.source() {
        writeln!(&mut message, "  caused by: {source}").unwrap();
        error = source;
    }
    message
}

pub(crate) fn multiline_array(
    elements: impl Iterator<Item = impl Into<toml_edit::Value>>,
) -> toml_edit::Array {
    let mut array: toml_edit::Array = elements
        .map(|item| {
            let mut value = item.into();
            value.decor_mut().set_prefix("\n    ");
            value
        })
        .collect();
    array.set_trailing_comma(true);
    array.set_trailing("\n");
    array
}

pub fn sha256_lowercase_hex(data: impl AsRef<[u8]>) -> String {
    lowercase_hex(Sha256::digest(data))
}

/// Encode `bytes` as lowercase hex string
pub fn lowercase_hex(bytes: Array<u8, typenum::U32>) -> String {
    hex::encode(bytes)
}

/// Return the deduplicated, in-order list of SPDX identifiers (licenses plus
/// any `WITH` exceptions) named in `expression`. Each identifier maps to a
/// `LICENSES/<id>.txt` file under REUSE conventions; the `+` "or later"
/// modifier does not affect the filename.
pub(crate) fn license_file_stems(expression: &spdx::Expression) -> IndexSet<String> {
    let mut stems: IndexSet<String> = IndexSet::new();
    for req in expression.requirements() {
        let license_name = match &req.req.license {
            spdx::LicenseItem::Spdx { id, .. } => id.name.to_string(),
            spdx::LicenseItem::Other(license_ref) => license_ref.to_string(),
        };
        stems.insert(license_name);

        if let Some(addition) = &req.req.addition {
            let addition_name = match addition {
                spdx::AdditionItem::Spdx(id) => id.name.to_string(),
                spdx::AdditionItem::Other(add_ref) => add_ref.to_string(),
            };
            stems.insert(addition_name);
        }
    }
    stems
}

#[derive(Error, Debug)]
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
    #[error("path `{path:?}` contains a NULL character")]
    ContainsNull { path: String },
    #[error(
        "path `{path}` is a directory path\n\
        (ends with a separator `/`), but a file path was expected"
    )]
    DirectoryPath { path: String },
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
    /// No relative components
    SubDirectory,
    /// No restrictions
    Directory,
}

impl RelativePathKind {
    fn is_file(self) -> bool {
        match self {
            RelativePathKind::SubFile | RelativePathKind::File => true,
            RelativePathKind::SubDirectory | RelativePathKind::Directory => false,
        }
    }

    fn allow_relative_components(self) -> bool {
        match self {
            RelativePathKind::SubFile | RelativePathKind::SubDirectory => false,
            RelativePathKind::File | RelativePathKind::Directory => true,
        }
    }
}

// TODO: use this in config and env validation

/// Parse a relative file path that uses `/` as separator.
/// If `allow_relative_components == true`, components `.` and `..` are allowed.
/// Trailing slash is not allowed.
pub fn parse_relative_unix_path(
    path: &str,
    kind: RelativePathKind,
) -> Result<&Utf8UnixPath, RelativeUnixPathError> {
    // Check for Windows and Unix absolute paths
    if Utf8WindowsPath::new(&path).is_absolute() || path.starts_with('/') {
        return Err(RelativeUnixPathError::Absolute {
            path: path.to_owned(),
        });
    }

    for c in path.bytes() {
        if c == b'\0' {
            return Err(RelativeUnixPathError::ContainsNull {
                path: path.to_owned(),
            });
        } else if c == b'\\' {
            return Err(RelativeUnixPathError::ContainsBackslash {
                path: path.to_owned(),
            });
        }
    }
    let stripped = if let Some(s) = path.strip_suffix('/') {
        if kind.is_file() {
            return Err(RelativeUnixPathError::DirectoryPath {
                path: path.to_owned(),
            });
        } else {
            s
        }
    } else {
        path
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
