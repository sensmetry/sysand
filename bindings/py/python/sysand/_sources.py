# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from __future__ import annotations
from typing import List
from pathlib import Path

import sysand._sysand_core as sysand_rs  # type: ignore


def sources(
    path: str | Path, *, include_deps: bool = True, env_path: str | Path | None = None
) -> List[Path]:
    if env_path is not None:
        env_path = str(env_path)

    return sysand_rs.do_sources_project_py(str(path), include_deps, env_path)  # type: ignore


__all__ = [
    "sources",
]
