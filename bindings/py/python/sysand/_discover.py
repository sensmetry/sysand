# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from pathlib import Path

import sysand._sysand_core as sysand_rs  # type: ignore

from ._model import Discovery


def discover(path: str | Path = ".") -> Discovery:
    """Find the project and the workspace enclosing ``path``.

    This is the same lookup every CLI command performs first: walk up from
    ``path`` to the nearest directory holding ``.project.json`` (or
    ``.meta.json``), and separately to the nearest one holding
    ``.workspace.json``. Paths are canonicalized, as :func:`sysand.root` does.
    A tool that must not operate inside a workspace checks
    ``workspace_root`` before doing anything else.

    Raises:
        ProjectError: a directory could not be read, or ``.workspace.json`` is
            malformed.
    """
    project_root, workspace_root = sysand_rs.do_discover_py(str(path))
    return Discovery(project_root=project_root, workspace_root=workspace_root)


__all__ = ["discover"]
