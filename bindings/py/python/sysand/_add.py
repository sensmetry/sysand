# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from __future__ import annotations

import sysand._sysand_core as sysand_rs  # type: ignore

from pathlib import Path


def add(path: Path | str, iri: str, version: str | None = None) -> None:
    sysand_rs.do_add_py(str(path), iri, version)


__all__ = ["add"]
