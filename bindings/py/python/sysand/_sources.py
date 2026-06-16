# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025-2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations
from typing import List
from pathlib import Path

from sysand._model import Dependencies
import sysand._sysand_core as sysand_rs  # type: ignore


def sources(
    path: str | Path,
    *,
    no_own: bool = False,
    dependencies: Dependencies = Dependencies.NONE,
    env_path: str | Path | None = None,
) -> List[Path]:
    """List the source files of the project at ``path``.

    By default only the project's own sources are listed. ``no_own`` excludes
    them, and ``dependencies`` selects which dependency sources to add. Every
    combination of ``no_own`` and ``dependencies`` is valid.

    Args:
        path: Path to the project.
        no_own: Exclude the project's own sources.
        dependencies: Which dependency sources to list (see :class:`Dependencies`).
            Defaults to :attr:`Dependencies.NONE` (no dependencies).
        env_path: Path to the environment in which dependencies are installed.
            Required unless ``dependencies`` is :attr:`Dependencies.NONE`.

    Returns:
        The source file paths as a list of :class:`~pathlib.Path`.
    """
    if env_path is not None:
        env_path = str(env_path)

    return sysand_rs.do_sources_project_py(  # type: ignore
        str(path), no_own, dependencies.name, env_path
    )


__all__ = [
    "sources",
]
