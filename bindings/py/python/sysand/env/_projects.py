# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from pathlib import Path

import sysand._sysand_core as sysand_rs  # type: ignore

from sysand._model import EnvProject


def projects(env_path: str | Path) -> list[EnvProject]:
    """List the projects recorded in the environment at ``env_path``.

    This reads ``env.toml``; install paths are returned verbatim (see
    :class:`~sysand.EnvProject`), so a caller re-reading the file set after a
    ``sync`` joins ``path`` onto the environment directory itself (or onto the
    workspace/project root for ``editable`` entries).

    Raises:
        EnvError: the environment is missing, unreadable or malformed.
    """
    return sysand_rs.do_env_projects_py(str(env_path))  # type: ignore


__all__ = ["projects"]
