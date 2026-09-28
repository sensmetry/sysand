# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025-2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations
from typing import List
from pathlib import Path

from ._model import Dependencies
from . import _sysand_core as sysand_rs


def sources(
    *,
    project_dir: str | Path,
    no_own: bool = False,
    dependencies: Dependencies = Dependencies.DEPS,
    env_path: str | Path | None = None,
) -> List[Path]:
    """List the source files of the project in ``project_dir``.

    By default, as with ``sysand sources``, the project's own sources are
    listed, followed by those of its dependencies other than the standard
    libraries. ``no_own`` excludes the project's own sources, and
    ``dependencies`` selects which dependency sources to add. Every
    combination of ``no_own`` and ``dependencies`` is valid.

    Args:
        project_dir: The project directory, the one holding ``.project.json``.
        no_own: Exclude the project's own sources.
        dependencies: Which dependency sources to list (see :class:`Dependencies`).
            Defaults to :attr:`Dependencies.DEPS` (dependencies, without the
            standard libraries).
        env_path: Path to the environment in which dependencies are installed.
            Defaults, as in the CLI, to the ``.sysand`` of the project's
            workspace, or else of the project itself. Without one, only a
            project with no dependencies to list succeeds.

    Returns:
        The source file paths as a list of :class:`~pathlib.Path`.
    """
    if env_path is not None:
        env_path = str(env_path)

    return sysand_rs.do_sources_project_py(  # type: ignore
        str(project_dir), no_own, dependencies.name, env_path
    )


__all__ = [
    "sources",
]
