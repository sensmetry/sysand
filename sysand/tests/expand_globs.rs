// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `expand_globs` does for an argument list what a POSIX shell does for a
//! command line, on Windows, where nothing else does.
//!
//! The binary and the Python binding both go through it.

use std::ffi::OsString;

use sysand::expand_globs;

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|&arg| arg.to_owned()).collect()
}

/// The shell has already expanded whatever it was going to.
#[cfg(not(windows))]
#[test]
fn leaves_arguments_alone_off_windows() {
    let args = ["sysand", "include", "*.sysml", "p?.sysml", "a[1].sysml"];
    assert_eq!(expand_globs(strings(&args)), os(&args));
}

/// Invalid Unicode is passed on, for clap to report.
#[cfg(unix)]
#[test]
fn passes_invalid_unicode_through() {
    use std::os::unix::ffi::OsStringExt as _;

    let arg = OsString::from_vec(b"\xff*.sysml".to_vec());
    assert_eq!(expand_globs([arg.clone()]), vec![arg]);
}

#[cfg(windows)]
mod windows {
    use camino_tempfile::tempdir;

    use std::ffi::OsString;

    use super::{expand_globs, os, strings};

    #[test]
    fn expands_star_and_question_mark() {
        let dir = tempdir().unwrap();
        for name in ["p1.sysml", "p2.sysml", "readme.txt"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let star = dir.path().join("*.sysml");
        let question = dir.path().join("p?.sysml");

        let expanded = expand_globs(strings(&["include", star.as_str()]));
        let p1 = dir.path().join("p1.sysml");
        let p2 = dir.path().join("p2.sysml");
        assert_eq!(expanded, os(&["include", p1.as_str(), p2.as_str()]));

        assert_eq!(
            expand_globs(strings(&[question.as_str()])),
            os(&[p1.as_str(), p2.as_str()])
        );
    }

    #[test]
    fn matches_case_insensitively() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("P1.SYSML"), "").unwrap();
        let pattern = dir.path().join("*.sysml");

        assert_eq!(
            expand_globs(strings(&[pattern.as_str()])),
            os(&[dir.path().join("P1.SYSML").as_str()])
        );
    }

    /// Brackets are ordinary characters in a Windows file name.
    #[test]
    fn treats_brackets_literally() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a[1].sysml"), "").unwrap();
        std::fs::write(dir.path().join("a1.sysml"), "").unwrap();
        let pattern = dir.path().join("a[1]*");

        assert_eq!(
            expand_globs(strings(&[pattern.as_str()])),
            os(&[dir.path().join("a[1].sysml").as_str()])
        );
    }

    #[test]
    fn keeps_a_pattern_that_matches_nothing() {
        let dir = tempdir().unwrap();
        let pattern = dir.path().join("*.sysml");

        assert_eq!(
            expand_globs(strings(&[pattern.as_str()])),
            os(&[pattern.as_str()])
        );
    }

    /// Invalid Unicode is passed on, for clap to report.
    #[test]
    fn passes_invalid_unicode_through() {
        use std::os::windows::ffi::OsStringExt as _;

        // An unpaired surrogate, then `*`
        let arg = OsString::from_wide(&[0xD800, u16::from(b'*')]);
        assert_eq!(expand_globs([arg.clone()]), vec![arg]);
    }

    #[test]
    fn leaves_arguments_without_wildcards_alone() {
        let args = ["sysand", "include", "a[1].sysml", "--name", "x"];
        assert_eq!(expand_globs(strings(&args)), os(&args));
    }
}
