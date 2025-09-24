# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from __future__ import annotations
from typing import List
from pathlib import Path

import sysand._sysand_core as sysand_rs  # type: ignore


def sources(
    env_path: str | Path,
    iri: str,
    version: str | None = None,
    *,
    include_deps: bool = True,
    include_std: bool = False,
) -> List[Path]:
    return sysand_rs.do_sources_env_py(  # type: ignore
        str(env_path), iri, version, include_deps, include_std
    )


__all__ = [
    "sources",
]
