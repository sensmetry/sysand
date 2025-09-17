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
) -> List[Path]:
    return sysand_rs.do_sources_env_py(str(env_path), iri, version, include_deps)  # type: ignore


__all__ = [
    "sources",
]
