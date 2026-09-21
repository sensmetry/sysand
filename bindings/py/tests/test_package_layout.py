# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""The shipped Python tree must not name its own top-level package.

Consumers vendor `python/sysand/` under a parent package of their own, so an
absolute `sysand.…` import in it either fails or — worse — silently binds a
*different* installation of this package. Checked against the source tree rather
than the installed copy, because the drop-in property is a property of what we
ship.
"""

from __future__ import annotations

import ast
from pathlib import Path

PACKAGE = Path(__file__).parents[1] / "python" / "sysand"


def _offenders(path: Path) -> list[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    found = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                if alias.name == "sysand" or alias.name.startswith("sysand."):
                    found.append(f"{path.name}:{node.lineno}: import {alias.name}")
        elif isinstance(node, ast.ImportFrom):
            module = node.module or ""
            if node.level == 0 and (module == "sysand" or module.startswith("sysand.")):
                found.append(f"{path.name}:{node.lineno}: from {module} import …")
    return found


def test_no_absolute_self_imports() -> None:
    assert PACKAGE.is_dir(), f"package source not found at {PACKAGE}"

    offenders = [
        offender
        for source in sorted(PACKAGE.rglob("*.py"))
        for offender in _offenders(source)
    ]

    assert offenders == [], (
        "the shipped tree must import itself relatively so it can be "
        "vendored under another parent package:\n  " + "\n  ".join(offenders)
    )
