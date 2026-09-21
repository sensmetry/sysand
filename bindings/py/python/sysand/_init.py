# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from . import _sysand_core as sysand_rs

from pathlib import Path


def init(*, project_dir: str | Path, name: str, publisher: str, version: str) -> None:
    """Create a project named ``name`` in ``project_dir``, creating the
    directory if it does not exist.

    Args:
        project_dir: The directory to write ``.project.json`` and
            ``.meta.json`` to. Its parent must exist.
        name: The project's name.
        publisher: The project's publisher.
        version: The project's version, a semver version such as ``"1.0.0"``.
    """
    if not Path(project_dir).exists():
        Path(project_dir).mkdir()

    sysand_rs.do_init_py_local_file(name, publisher, version, str(project_dir))


__all__ = ["init"]
