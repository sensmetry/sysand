# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing
from pathlib import Path

import sysand._sysand_core as sysand_rs  # type: ignore

from ._auth import AuthPolicy, Resolution
from ._model import LockResult, ProvidedProject, SyncedProject, SyncOutcome


def _synced(
    entries: typing.Iterable[tuple[str, str, str | None]],
) -> list[SyncedProject]:
    return [
        SyncedProject(iri=iri, version=version, path=path)
        for iri, version, path in entries
    ]


def sync(
    path: str | Path = ".",
    *,
    lock: LockResult | str | None = None,
    resolution: Resolution | None = None,
    auth: AuthPolicy | None = None,
    provided: typing.Sequence[ProvidedProject] | None = None,
    no_prune: bool = False,
) -> SyncOutcome:
    """Install what the lockfile says into the project's ``.sysand``
    environment, as ``sysand sync`` does, and report every change.

    ``path`` is where discovery starts (see :func:`lock`). Unlike the CLI,
    a missing lockfile is an error, not an implicit ``lock``: the caller is
    expected to have seen the resolution first. ``lock`` may be the
    :class:`LockResult` of a previous :func:`lock` call (its ``text`` is
    used) or lockfile text; by default ``sysand-lock.toml`` is read from the
    project root.

    Projects no longer in the lockfile are removed from the environment
    unless ``no_prune`` is set.

    Raises:
        ProjectError: not inside a project, or no lockfile.
        SyncError: installation failed part-way; ``partial`` lists what was
            installed and pruned before the failure and ``wrote`` says
            whether anything was.
        EnvError: the environment could not be read or written.
    """
    if resolution is None:
        resolution = Resolution()
    lock_text: str | None
    if lock is None:
        lock_text = None
    elif isinstance(lock, str):
        lock_text = lock
    else:
        lock_text = lock["text"]
    installed, pruned, kept = sysand_rs.do_sync_py(
        str(path),
        lock_text,
        resolution._spec(),
        auth._spec() if auth is not None else None,
        list(provided or ()),
        no_prune,
    )
    return SyncOutcome(
        installed=_synced(installed), pruned=_synced(pruned), kept=_synced(kept)
    )


__all__ = ["sync"]
