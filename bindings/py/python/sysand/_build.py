# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from ._model import CompressionMethod
from . import _sysand_core as sysand_rs

from pathlib import Path


def build(
    *,
    output_path: str | Path,
    project_dir: str | Path | None = None,
    compression: CompressionMethod | None = None,
) -> None:
    """Build a KerML Project Archive (KPAR) of the project in
    ``project_dir``.

    Args:
        output_path: The KPAR file to write.
        project_dir: The project directory, the one holding ``.project.json``
            and ``.meta.json``. It is required for now: without it the call
            raises :class:`NotImplementedError`.
        compression: How to compress the archive's entries. Defaults to
            :attr:`CompressionMethod.DEFLATED`.
    """
    if project_dir is not None:
        project_dir = str(project_dir)

    # comp = None if compression is None else _convert_compression(compression)
    comp = None if compression is None else compression.name
    sysand_rs.do_build_py(str(output_path), project_dir, comp)


__all__ = [
    "build",
]
