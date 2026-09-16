# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from . import _sysand_core as sysand_rs

from pathlib import Path


def init(
    *,
    project_dir: str | Path,
    name: str,
    publisher: str,
    version: str,
    metamodel: str,
    license: str | None = None,
) -> None:
    """Create a ``model.toml`` project named ``name`` in ``project_dir``,
    creating the directory if it does not exist.

    ``metamodel`` is required: a project declares one and it cannot be
    guessed. It is one of ``https://www.omg.org/spec/KerML/<date>``,
    ``https://www.omg.org/spec/SysML/<date>``,
    ``https://www.sysand.com/spec/KerML/<date>`` or
    ``https://www.sysand.com/spec/SysML/<date>``. The workspace
    exception ``sysand init`` has -- inheriting from the default group
    -- belongs to the command, which knows the scope it was run in.

    Args:
        project_dir: The directory to write ``model.toml`` to. Its parent
            must exist.
        name: The project's name.
        publisher: The project's publisher.
        version: The project's version, a semver version such as ``"1.0.0"``.
        metamodel: The project's metamodel, an IRI such as
            ``"https://www.omg.org/spec/SysML/20250201"``.
        license: The project's license, an SPDX license expression such as
            ``"MIT OR Apache-2.0"``. Omitted from the manifest when ``None``.

    Raises:
        ValueError: ``name``, ``publisher``, ``version``, ``license`` or
            ``metamodel`` is not valid.
    """
    if not Path(project_dir).exists():
        Path(project_dir).mkdir()

    sysand_rs.do_init_py_local_file(
        name, publisher, version, str(project_dir), metamodel, license
    )


__all__ = ["init"]
