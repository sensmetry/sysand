# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from . import _sysand_core as sysand_rs

from pathlib import Path


def exclude(
    *,
    project_dir: Path | str,
    src_path: str | Path,
) -> None:
    """Exclude a source file from the ``.meta.json`` of the project in
    ``project_dir``.

    Args:
        project_dir: The project directory, the one holding ``.meta.json``.
        src_path: The file to exclude, relative to ``project_dir``, with
            ``/`` as separator. It is not normalized.
    """
    sysand_rs.do_exclude_py(str(project_dir), str(src_path))


__all__ = ["exclude"]
