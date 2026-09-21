# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing
from pathlib import Path

from . import _sysand_core as sysand_rs

from ._auth import AuthPolicy, Resolution
from ._model import EnvProject, LockResult, ProvidedProject, SyncedProject, SyncOutcome


def _with_paths(
    entries: typing.Iterable[dict[str, str]],
    paths: typing.Mapping[tuple[str, str], str],
) -> list[SyncedProject]:
    return [
        SyncedProject(
            iri=entry["iri"],
            version=entry["version"],
            path=paths.get((entry["iri"], entry["version"])),
        )
        for entry in entries
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
    project or workspace root.

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
    outcome, projects = sysand_rs.do_sync_py(
        str(path),
        lock_text,
        resolution._spec(),
        auth._spec() if auth is not None else None,
        list(provided or ()),
        no_prune,
    )
    # The environment's entries after the sync, under every identifier; a
    # pruned entry is gone from them, so its path is ``None``.
    env_projects: list[EnvProject] = projects
    paths = {
        (iri, project["version"]): project["path"]
        for project in env_projects
        for iri in project["identifiers"]
    }
    return SyncOutcome(
        installed=_with_paths(outcome["installed"], paths),
        pruned=_with_paths(outcome["pruned"], paths),
        kept=_with_paths(outcome["kept"], paths),
    )


__all__ = ["sync"]
