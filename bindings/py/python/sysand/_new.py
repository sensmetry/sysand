# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from __future__ import annotations

import sysand._sysand_core as sysand_rs  # type: ignore

from pathlib import Path


def init(name: str, publisher: str, version: str, path: str | Path = ".") -> None:
    sysand_rs.do_new_py_local_file(name, publisher, version, str(path))


def new(name: str, publisher: str, version: str, path: str | Path = ".") -> None:
    if not Path(path).exists():
        Path(path).mkdir()

    sysand_rs.do_new_py_local_file(name, publisher, version, str(path))


__all__ = [
    "init",
    "new",
]
