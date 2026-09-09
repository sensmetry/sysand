# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import sysand._sysand_core as sysand_rs  # type: ignore

from pathlib import Path


def add(path: Path | str, iri: str, version: str | None = None) -> bool:
    """Add a usage of ``iri`` (an IRI or ``publisher/name`` shorthand) to the
    project at ``path``.

    Returns:
        ``True`` when a new usage was added, ``False`` when ``iri`` was
        already declared and the call merged into (or ignored for) the
        existing usage.
    """
    return sysand_rs.do_add_py(str(path), iri, version)  # type: ignore


__all__ = ["add"]
