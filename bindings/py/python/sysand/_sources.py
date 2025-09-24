# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from __future__ import annotations
from typing import List
from pathlib import Path

import sysand._sysand_core as sysand_rs  # type: ignore


def sources(
    path: str | Path,
    *,
    include_deps: bool = True,
    env_path: str | Path | None = None,
    include_std: bool = False,
) -> List[Path]:
    if env_path is not None:
        env_path = str(env_path)

    return sysand_rs.do_sources_project_py(  # type: ignore
        str(path), include_deps, env_path, include_std
    )


__all__ = [
    "sources",
]
