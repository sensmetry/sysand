# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import sysand._sysand_core as sysand_rs  # type: ignore

from pathlib import Path


def remove(path: Path | str, iri: str) -> None:
    sysand_rs.do_remove_py(str(path), iri)


__all__ = ["remove"]
