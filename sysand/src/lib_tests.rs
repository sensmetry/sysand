// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::quote_for_shell;

#[cfg(unix)]
mod unix {
    use super::quote_for_shell;

    #[test]
    fn empty_string() {
        assert_eq!(quote_for_shell(""), "''");
    }

    #[test]
    fn plain_word_is_wrapped_but_unchanged() {
        assert_eq!(quote_for_shell("hello"), "'hello'");
    }

    #[test]
    fn spaces_are_preserved_literally() {
        assert_eq!(quote_for_shell("hello world"), "'hello world'");
    }

    #[test]
    fn single_quote_is_escaped() {
        // ' -> '\''
        assert_eq!(quote_for_shell("it's"), r"'it'\''s'");
    }

    #[test]
    fn leading_single_quote() {
        assert_eq!(quote_for_shell("'foo"), r"''\''foo'");
    }

    #[test]
    fn trailing_single_quote() {
        assert_eq!(quote_for_shell("foo'"), r"'foo'\'''");
    }

    #[test]
    fn only_a_single_quote() {
        assert_eq!(quote_for_shell("'"), r"''\'''");
    }

    #[test]
    fn multiple_single_quotes() {
        assert_eq!(quote_for_shell("''"), r"''\'''\'''");
    }

    #[test]
    fn shell_metacharacters_are_inert_inside_single_quotes() {
        let arg = r"$HOME `cmd` \ ; | & > < # ~ * ? [ ] ( ) { } !";
        let quoted = quote_for_shell(arg);
        assert_eq!(quoted, format!("'{arg}'"));
    }

    #[test]
    fn backslash_is_left_untouched() {
        assert_eq!(quote_for_shell(r"a\b"), r"'a\b'");
    }

    #[test]
    fn newline_is_preserved_literally() {
        assert_eq!(quote_for_shell("a\nb"), "'a\nb'");
    }

    #[test]
    fn non_ascii_is_preserved() {
        assert_eq!(quote_for_shell("héllo→世界"), "'héllo→世界'");
    }

    #[test]
    fn round_trips_through_sh_c() {
        // Feed a handful of adversarial strings through `sh -c 'printf %s ...'`
        // and check that the shell reconstructs exactly the original argument.
        use std::process::Command;

        let cases = [
            "",
            "plain",
            "has space",
            "it's",
            "'",
            "''",
            "'''",
            r#"$(rm -rf /) `echo hi` "quoted" \n"#,
            "trailing'",
            "'leading",
        ];

        for case in cases {
            let quoted = quote_for_shell(case);
            let script = format!("printf '%s' {quoted}");
            let output = Command::new("sh")
                .arg("-c")
                .arg(&script)
                .output()
                .expect("failed to run sh");
            assert!(
                output.status.success(),
                "sh failed for case {case:?} with script {script:?}: {output:?}"
            );
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                case,
                "round trip mismatch for case {case:?}, script was {script:?}"
            );
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::quote_for_shell;

    #[test]
    fn empty_string() {
        assert_eq!(quote_for_shell(""), "\"\"");
    }

    #[test]
    fn plain_word_is_wrapped_but_unchanged() {
        assert_eq!(quote_for_shell("hello"), "\"hello\"");
    }

    #[test]
    fn spaces_are_preserved_literally() {
        assert_eq!(quote_for_shell("hello world"), "\"hello world\"");
    }

    #[test]
    fn quote_is_escaped_with_one_backslash() {
        // no preceding backslashes: " -> \"
        assert_eq!(quote_for_shell(r#"a"b"#), r#""a\"b""#);
    }

    #[test]
    fn backslash_before_quote_is_doubled_and_escaped() {
        // one preceding backslash: N=1 -> 2*1+1 = 3 backslashes then the quote
        assert_eq!(quote_for_shell(r#"a\"b"#), r#""a\\\"b""#);
    }

    #[test]
    fn backslash_not_before_quote_is_left_as_is() {
        assert_eq!(quote_for_shell(r"a\b"), r#""a\b""#);
    }

    #[test]
    fn trailing_backslashes_are_doubled() {
        assert_eq!(quote_for_shell(r"a\"), r#""a\\""#);
        assert_eq!(quote_for_shell(r"a\\"), r#""a\\\\""#);
    }

    #[test]
    fn only_backslashes() {
        assert_eq!(quote_for_shell(r"\"), r#""\\""#);
        assert_eq!(quote_for_shell(r"\\"), r#""\\\\""#);
    }

    #[test]
    fn only_a_single_quote_char() {
        assert_eq!(quote_for_shell("\""), "\"\\\"\"");
    }

    #[test]
    fn multiple_quote_chars_with_no_backslashes() {
        assert_eq!(quote_for_shell("\"\"\""), "\"\\\"\\\"\\\"\"");
    }

    #[test]
    fn non_ascii_is_preserved() {
        assert_eq!(quote_for_shell("héllo→世界"), "\"héllo→世界\"");
    }
}

/// The crash hook belongs to whoever owns the process.
///
/// A panic hook is process-wide, so the one test that installs it runs in a
/// child process: the test binary, re-executed to run only that test, with
/// [`CHILD`] set to tell it to be the child. The parent reads what the child
/// printed.
mod panic_hook {
    use std::process::Command;

    use crate::{ProcessOwnership, lib_main_with};

    const CHILD: &str = "SYSAND_TEST_PANIC_HOOK_CHILD";
    const BANNER: &str = "Sysand crashed";
    const HOST_PANIC: &str = "the host panicked";

    /// Run the CLI twice as `ownership` would, then panic as the host.
    ///
    /// `--version` is answered during parsing, so neither run touches the file
    /// system or the logger; what is left is whatever the entry point does to
    /// the process before parsing.
    fn be_the_child(ownership: ProcessOwnership) {
        for _ in 0..2 {
            assert_eq!(lib_main_with(["sysand", "--version"], ownership), 0);
        }
        panic!("{HOST_PANIC}");
    }

    /// Re-run the test `name` in a child process and return its stderr.
    fn stderr_of_child(name: &str) -> String {
        let out = Command::new(std::env::current_exe().expect("test binary path"))
            .args(["--exact", name, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output()
            .expect("failed to run the test binary");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(!out.status.success(), "the child did not panic: {stderr}");
        assert!(stderr.contains(HOST_PANIC), "{stderr}");
        stderr
    }

    #[test]
    fn an_embedded_run_leaves_the_hosts_panics_alone() {
        if std::env::var_os(CHILD).is_some() {
            be_the_child(ProcessOwnership::Embedded);
        }
        let stderr =
            stderr_of_child("tests::panic_hook::an_embedded_run_leaves_the_hosts_panics_alone");
        // A panic in the host's own code is the host's to report, not a
        // Sysand bug.
        assert!(!stderr.contains(BANNER), "{stderr}");
    }

    // The hook is not installed in debug builds
    #[cfg(not(debug_assertions))]
    #[test]
    fn an_owning_run_reports_a_crash_once() {
        if std::env::var_os(CHILD).is_some() {
            be_the_child(ProcessOwnership::Owned);
        }
        let stderr = stderr_of_child("tests::panic_hook::an_owning_run_reports_a_crash_once");
        // Two runs, one banner: running the CLI again does not stack another
        // copy of the hook on top of the first.
        assert_eq!(stderr.matches(BANNER).count(), 1, "{stderr}");
    }
}
