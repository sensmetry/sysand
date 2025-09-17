# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from __future__ import annotations

import sysand._sysand_core as sysand_rs  # type: ignore

from pathlib import Path


def install_path(env_path: str | Path, iri: str, location: str | Path) -> None:
    sysand_rs.do_env_install_path_py(str(env_path), iri, str(location))


__all__ = ["install_path"]
