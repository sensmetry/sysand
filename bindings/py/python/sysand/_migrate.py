# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from pathlib import Path

from . import _sysand_core as sysand_rs

from ._auth import AuthPolicy, Resolution


def migrate(
    path: str | Path = ".",
    publisher: str | None = None,
    metamodel: str | None = None,
    allow_dirty: bool = False,
    allow_no_vcs: bool = False,
    resolution: Resolution | None = None,
    auth: AuthPolicy | None = None,
) -> None:
    """Convert a legacy json project into a ``model.toml`` one.

    Writes ``model.toml`` and then removes ``.project.json`` and
    ``.meta.json``, as ``sysand migrate`` does. A project whose
    ``.project.json`` declares no ``publisher`` needs one here, since
    ``model.toml`` requires it. So does a project whose ``.meta.json``
    declares no ``metamodel``, or one Sysand does not recognize:
    ``metamodel`` supplies it, and it is one of the four values a
    ``model.toml`` holds.

    The working tree is checked first, because the json manifests are
    deleted: uncommitted changes and a directory outside any git
    repository are both refused unless the matching flag is passed.

    An ``http(s)`` usage, and a ``pkg:sysand`` usage with no version
    constraint, are resolved through an index -- so ``resolution`` and
    ``auth`` take the same shapes as :func:`sysand.lock`'s.

    Raises:
        ProjectError: the project could not be read or converted, the
            working tree is dirty, or it is not inside a git work tree.
    """
    sysand_rs.do_migrate_py(
        str(path), publisher, metamodel, allow_dirty, allow_no_vcs, resolution, auth
    )


__all__ = ["migrate"]
