# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025-2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations
from typing import List
from pathlib import Path

from .._model import Dependencies
from .. import _sysand_core as sysand_rs


def sources(
    *,
    env_path: str | Path,
    iri: str,
    version: str | None = None,
    include_own: bool = True,
    dependencies: Dependencies = Dependencies.DEPS,
) -> List[Path]:
    """List the source files of an (already installed) project in an environment.

    By default, as with ``sysand env sources``, the project's own sources are
    listed, followed by those of its dependencies other than the standard
    libraries. ``include_own=False`` excludes the project's own sources, and
    ``dependencies`` selects which dependency sources to add. Every
    combination of ``include_own`` and ``dependencies`` is valid.

    Args:
        env_path: Path to the environment in which the project is installed.
        iri: IRI of the installed project to list sources for.
        version: Version constraint selecting which installed project to use.
            Defaults to the first matching candidate.
        include_own: Include the project's own sources.
        dependencies: Which dependency sources to list (see :class:`Dependencies`).
            Defaults to :attr:`Dependencies.DEPS` (dependencies, without the
            standard libraries).

    Returns:
        The source file paths as a list of :class:`~pathlib.Path`.

    Raises:
        ValueError: ``iri`` is not an IRI (``publisher/name`` is not one), or
            ``version`` is not a semver requirement.
        NotFoundError: no installed version of the project (matching
            ``version``) is in the environment.
        EnvError: the environment is missing, unreadable or malformed, or
            the installed project or its dependencies are (the environment
            may need a :func:`~sysand.sync`).
    """
    return sysand_rs.do_sources_env_py(  # type: ignore
        str(env_path), iri, version, include_own, dependencies.name
    )


__all__ = [
    "sources",
]
