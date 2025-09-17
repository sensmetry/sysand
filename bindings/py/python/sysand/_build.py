from __future__ import annotations

import sysand._sysand_core as sysand_rs  # type: ignore

from pathlib import Path


def build(output_path: str | Path, project_path: str | Path | None = None) -> None:
    if project_path is not None:
        project_path = str(project_path)
    sysand_rs.do_build_py(str(output_path), project_path)


__all__ = [
    "build",
]
