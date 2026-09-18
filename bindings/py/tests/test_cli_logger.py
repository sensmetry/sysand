# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`_run_cli` does not complain about a logger the host installed first.

`log` allows one logger per process. The binding functions install the
`pyo3-log` bridge and the CLI installs `env_logger`, so in a process that uses
both the second one loses.

The binary is right to report that: nothing else runs in its process, so a
taken slot is a surprise. In the host's process it is the arrangement the host
chose, and saying so on every invocation is noise. `_run_cli` therefore runs
the CLI as `ProcessOwnership::Embedded`.

Every test here must use a command that reaches `run_cli`. `--version` and
`--help` do not: clap answers them out of `try_parse_from`, before a logger is
ever installed, so they would pass whatever the ownership.

`capfd` rather than `capsys`: the warning is Rust's `eprintln!`, which writes
to file descriptor 2 without passing through `sys.stderr`.
"""

from __future__ import annotations

import logging
from pathlib import Path

import pytest

# The session-wide `_claim_logger` fixture in `conftest.py` has already taken
# the global slot for `pyo3-log`, which is exactly the state under test.
from mockindex import run_cli_in

WARNING = "failed to set up logger"


def test_no_logger_warning_on_stderr(
    capfd: pytest.CaptureFixture[str], tmp_path: Path
) -> None:
    # A command, not a flag: this has to get past argument parsing and into
    # `run_cli`, which is where the logger is installed. It fails -- there is
    # no project in `tmp_path` -- and that is fine, the failure happens after.
    assert run_cli_in(tmp_path, "info", "--no-config") == 1

    assert WARNING not in capfd.readouterr().err


def test_the_command_still_reports_its_own_failure(
    capfd: pytest.CaptureFixture[str], tmp_path: Path
) -> None:
    # Silencing the warning must not silence the command: the error text that
    # follows it does not go through the logger, and has to survive.
    assert run_cli_in(tmp_path, "info", "--no-config") == 1

    captured = capfd.readouterr().err
    assert "error:" in captured, captured
    assert WARNING not in captured


def test_verbosity_still_reaches_the_host_logger(
    caplog: pytest.LogCaptureFixture, tmp_path: Path
) -> None:
    # What the warning was warning about, and the reason it can be dropped
    # rather than merely hidden: `--verbose` still applies, through whatever
    # logger the host installed. The records arrive as Python logging records,
    # because the bridge is what is installed.
    with caplog.at_level(logging.DEBUG):
        assert run_cli_in(tmp_path, "-v", "info", "--no-config") == 1

    assert any(record.levelno == logging.DEBUG for record in caplog.records), (
        f"no debug records reached the host logger: {caplog.records}"
    )
