# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`_run_cli` returns the exit code, not whether the run succeeded.

The CLI distinguishes three outcomes: 0, clap's usage code 2, and 1 for a
runtime failure. Note that ``bool`` is a subclass of ``int``, so
``_run_cli(...) == 0`` would hold for ``False``; only an assertion about
the *type* tells a boolean result from an exit code.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from mockindex import run_cli_in
from sysand._sysand_core import _run_cli  # type: ignore


def test_returns_an_int_not_a_bool() -> None:
    code = _run_cli(["sysand", "--version"])

    assert type(code) is int
    assert code == 0


def test_clap_usage_error_is_two(tmp_path: Path) -> None:
    assert run_cli_in(tmp_path, "--not-a-flag") == 2


def test_missing_subcommand_is_two(tmp_path: Path) -> None:
    assert run_cli_in(tmp_path) == 2


def test_runtime_failure_is_one(tmp_path: Path) -> None:
    # No project in the directory, so this fails while running, which a
    # boolean could not tell from a usage error.
    assert run_cli_in(tmp_path, "lock", "--no-config", "--no-index") == 1


def test_module_entry_point_propagates_the_code(tmp_path: Path) -> None:
    # `python -m sysand` is the user-visible half of this: a usage error
    # exits with 2, as the native binary does.
    result = subprocess.run(
        [sys.executable, "-m", "sysand", "--not-a-flag"],
        cwd=tmp_path,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 2, result.stderr.decode()
