# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from . import _sysand_core as sysand_rs

from pathlib import Path


def include(
    *,
    project_dir: str | Path,
    src_path: str | Path,
    compute_checksum: bool = False,
    index_symbols: bool = True,
) -> None:
    """Include a source file in the ``.meta.json`` of the project in
    ``project_dir``.

    Including a file that is already included updates its metadata, which is
    not updated automatically when the file changes.

    The file's language is chosen by its extension, so with ``index_symbols``
    a file that is neither ``.sysml`` nor ``.kerml`` is refused.

    Args:
        project_dir: The project directory, the one holding ``.meta.json``.
        src_path: The file to include, relative to ``project_dir``, with
            ``/`` as separator. It is not normalized.
        compute_checksum: Record the file's current SHA256 checksum.
        index_symbols: Add the file's top-level symbols to the index,
            replacing any it contributed before.
    """
    sysand_rs.do_include_py(
        str(project_dir), str(src_path), compute_checksum, index_symbols
    )


__all__ = ["include"]
