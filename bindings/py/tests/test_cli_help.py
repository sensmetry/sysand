# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`_render_long_help` returns clap's long help instead of printing it.

An embedding CLI grafts sysand on as a subcommand and has to *return* its
help text. clap writes to the process's file descriptors, so redirecting
``sys.stdout`` — the trick a pure-Python parser allows — captures nothing.
"""

from __future__ import annotations

from sysand._sysand_core import _render_long_help  # type: ignore


def test_root_help() -> None:
    help_text = _render_long_help("sysand", [])

    assert isinstance(help_text, str)
    assert "Usage: sysand" in help_text
    # The long about, not the short one.
    assert "https://docs.sysand.com/client/" in help_text


def test_program_name_and_subcommand_path() -> None:
    help_text = _render_long_help("custom sysand", ["env"])

    assert "Usage: custom sysand env" in help_text


def test_unknown_subcommand_returns_text_rather_than_raising() -> None:
    help_text = _render_long_help("sysand", ["definitely-not-a-subcommand"])

    assert "error:" in help_text
    assert "definitely-not-a-subcommand" in help_text
