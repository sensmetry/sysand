# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from . import _sysand_core as sysand_rs

from pathlib import Path


def exclude(
    *,
    path: Path | str,
    src_path: str | Path,
) -> None:
    sysand_rs.do_exclude_py(str(path), str(src_path))


__all__ = ["exclude"]
