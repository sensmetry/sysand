# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from . import _sysand_core as sysand_rs

from pathlib import Path


def init(*, path: str | Path, name: str, publisher: str, version: str) -> None:
    """Create a project named ``name`` at ``path``, creating the directory
    if it does not exist.
    """
    if not Path(path).exists():
        Path(path).mkdir()

    sysand_rs.do_init_py_local_file(name, publisher, version, str(path))


__all__ = ["init"]
