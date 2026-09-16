# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from pathlib import Path

import sysand._sysand_core as sysand_rs  # type: ignore


def migrate(
    path: str | Path = ".",
    publisher: str | None = None,
    allow_dirty: bool = False,
    allow_no_vcs: bool = False,
) -> None:
    """Convert a legacy json project into a ``model.toml`` one.

    Writes ``model.toml`` and then removes ``.project.json`` and
    ``.meta.json``, as ``sysand migrate`` does. A project whose
    ``.project.json`` declares no ``publisher`` needs one here, since
    ``model.toml`` requires it.

    The working tree is checked first, because the json manifests are
    deleted: uncommitted changes and a directory outside any git
    repository are both refused unless the matching flag is passed.

    Raises:
        ProjectError: the project could not be read or converted, the
            working tree is dirty, or it is not inside a git work tree.
    """
    sysand_rs.do_migrate_py(str(path), publisher, allow_dirty, allow_no_vcs)


__all__ = ["migrate"]
