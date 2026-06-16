# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025-2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations
from typing import List
from pathlib import Path

from sysand._model import Dependencies
import sysand._sysand_core as sysand_rs  # type: ignore


def sources(
    env_path: str | Path,
    iri: str,
    version: str | None = None,
    *,
    no_own: bool = False,
    dependencies: Dependencies = Dependencies.NONE,
) -> List[Path]:
    """List the source files of an (already installed) project in an environment.

    By default only the project's own sources are listed. ``no_own`` excludes
    them, and ``dependencies`` selects which dependency sources to add. Every
    combination of ``no_own`` and ``dependencies`` is valid.

    Args:
        env_path: Path to the environment in which the project is installed.
        iri: IRI of the installed project to list sources for.
        version: Version constraint selecting which installed project to use.
            Defaults to the first matching candidate.
        no_own: Exclude the project's own sources.
        dependencies: Which dependency sources to list (see :class:`Dependencies`).
            Defaults to :attr:`Dependencies.NONE` (no dependencies).

    Returns:
        The source file paths as a list of :class:`~pathlib.Path`.
    """
    return sysand_rs.do_sources_env_py(  # type: ignore
        str(env_path), iri, version, no_own, dependencies.name
    )


__all__ = [
    "sources",
]
