# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025-2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from pathlib import Path

import sysand._sysand_core as sysand_rs  # type: ignore


def root(path: str | Path = ".") -> Path | None:
    """Find the root directory of the project containing ``path``.

    Args:
        path: Path to start searching from. Defaults to the current directory.

    Returns:
        The canonicalized project root as a :class:`~pathlib.Path`, or ``None``
        if ``path`` is not inside a project.
    """
    result = sysand_rs.do_root_py(str(path))
    return Path(result) if result is not None else None


__all__ = [
    "root",
]
