# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from __future__ import annotations

import sysand._sysand_core as sysand_rs  # type: ignore

from pathlib import Path
from typing import Literal


def include(
    path: str | Path,
    src_path: str | Path,
    *,
    compute_checksum: bool = False,
    index_symbols: bool = True,
    force_format: Literal["sysml", "kerml"] | None = None,
) -> None:
    sysand_rs.do_include_py(
        str(path), str(src_path), compute_checksum, index_symbols, force_format
    )


__all__ = ["include"]
