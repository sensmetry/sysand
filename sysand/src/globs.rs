// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::ffi::OsString;

/// Expand the glob patterns in an argument list, on Windows, where `cmd.exe`
/// and PowerShell leave that to the program.
///
/// The binary runs its own command line through this, and an embedder that
/// is handed its arguments as a list should do the same, so that both expand
/// patterns alike.
///
/// Only arguments containing `*` or `?` are expanded, whether or not they
/// were quoted: by the time the arguments are a list, the quotes are gone.
/// An argument that is not valid Unicode is never expanded. Matching ignores case,
/// `*` and `?` do not match a path separator, and `[` and `]` are literal
/// characters, as they are in Windows file names. A pattern that matches
/// nothing, or is not a valid pattern, is passed on unchanged, with a warning
/// on standard error in the latter case.
///
/// Elsewhere the shell has done this already, and the arguments are returned
/// as they are.
pub fn expand_globs<I, T>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    #[cfg(windows)]
    {
        use glob::{MatchOptions, glob_with};

        let options = MatchOptions {
            case_sensitive: false,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };

        args.into_iter()
            .flat_map(|arg| {
                let arg: OsString = arg.into();
                let Some(pattern) = arg.to_str().filter(|s| s.contains(['*', '?'])) else {
                    return vec![arg];
                };

                // Treat '[' and ']' as literal characters to match Windows behavior
                let escaped = pattern.replace('[', "[[]");

                match glob_with(&escaped, options) {
                    Ok(entries) => {
                        let matches: Vec<OsString> = entries
                            .filter_map(|m| match m {
                                Ok(s) => Some(s.into_os_string()),
                                Err(e) => {
                                    // can't use log::warn here, since the logger is likely uninitialized
                                    eprintln!("warning: failed to expand pattern: {e}");
                                    None
                                }
                            })
                            .collect();

                        if !matches.is_empty() {
                            return matches;
                        }
                    }
                    Err(e) => eprintln!("warning: invalid pattern `{pattern}`: {e}"),
                }

                vec![arg]
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        args.into_iter().map(Into::into).collect()
    }
}
