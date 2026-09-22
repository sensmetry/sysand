# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing
from pathlib import Path

from . import _sysand_core as sysand_rs

from ._auth import AuthPolicy, Resolution
from ._model import LockResult, ProvidedProject


def lock(
    *,
    path: str | Path = ".",
    resolution: Resolution | None = None,
    auth: AuthPolicy | None = None,
    provided: typing.Sequence[ProvidedProject] | None = None,
    write: bool = True,
) -> LockResult:
    """Resolve the project's dependencies into a lockfile, as ``sysand lock``
    does, and return the resolved set.

    ``path`` is where discovery starts, like the CLI's working directory:
    the enclosing project is found by walking up, and both the lockfile and
    the environment root are the project root.

    Inside a workspace this locks the *workspace*: the lockfile is written
    at the workspace root and every member is a root of the solve, so a
    member that nothing else depends on is still resolved.

    With ``write=True`` (the default, matching the CLI) ``sysand-lock.toml``
    is written at the project root; the returned ``text`` is exactly what
    was written. With ``write=False`` nothing is written and ``text`` is
    what would have been — call it first, show the user the delta, then
    lock again with ``write=True``.

    ``resolution`` defaults to :class:`Resolution` ``()`` (the CLI's
    semantics); ``auth`` defaults to :meth:`AuthPolicy.none`. ``provided``
    projects are treated as already present, like the standard libraries.

    Args:
        path: Where to start discovery. Defaults to the current directory.
        resolution: Where to look for dependencies.
        auth: How to authenticate to indexes.
        provided: Projects that are already present and never installed.
        write: Write ``sysand-lock.toml``.

    Returns:
        The resolved projects and the lockfile text.

    Raises:
        SolveError: no compatible set of versions exists. ``conflicts`` names
            every participant (which pin excludes what), ``report`` is the
            CLI's own text, ``wrote`` is ``False``.
        ProjectError: ``path`` is not inside a project, the project is
            malformed, or (with ``wrote=True``) the lockfile could not be
            written after a successful solve.
        AuthError, IndexProtocolError, ResolutionError: an index could not be
            used.
        LockHeldError: with ``write=True``, another process holds the
            workspace's advisory lock; nothing was written.
    """
    if resolution is None:
        resolution = Resolution()
    text, projects = sysand_rs.do_lock_py(
        str(path),
        resolution._spec(),
        auth._spec() if auth is not None else None,
        list(provided or ()),
        write,
    )
    return LockResult(projects=projects, text=text)


__all__ = ["lock"]
