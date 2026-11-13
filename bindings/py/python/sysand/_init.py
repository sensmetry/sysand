# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import enum
from pathlib import Path

from . import _sysand_core as sysand_rs


class ProjectKind(str, enum.Enum):
    """The kinds of project ``init`` creates."""

    MODEL_TOML = "model-toml"
    """A directory described by ``model.toml``."""

    PROJECT_JSON = "project-json"
    """A directory described by ``.project.json`` and ``.meta.json``."""


def init(
    *,
    project_dir: str | Path,
    name: str,
    publisher: str,
    version: str,
    metamodel: str,
    kind: ProjectKind,
    license: str | None = None,
) -> None:
    """Create a project named ``name`` in ``project_dir``, creating the
    directory if it does not exist.

    ``metamodel`` is required: a project declares one and it cannot be
    guessed. It is one of ``https://www.omg.org/spec/KerML/<date>``,
    ``https://www.omg.org/spec/SysML/<date>``,
    ``https://www.sysand.com/spec/KerML/<date>`` or
    ``https://www.sysand.com/spec/SysML/<date>``. The CLI's workspace
    exception, inheriting the metamodel from the default group, does not
    apply here.

    Args:
        project_dir: The directory to write the project to. Its parent
            must exist.
        name: The project's name.
        publisher: The project's publisher.
        version: The project's version, a semver version such as ``"1.0.0"``.
        metamodel: The project's metamodel, an IRI such as
            ``"https://www.omg.org/spec/SysML/20250201"``. A
            ``ProjectKind.PROJECT_JSON`` project takes an OMG one only.
        kind: The kind of project to create, such as
            ``ProjectKind.MODEL_TOML``.
        license: The project's license, an SPDX license expression such as
            ``"MIT OR Apache-2.0"``. Omitted from the project when ``None``.

    Raises:
        ValueError: ``name``, ``publisher``, ``version``, ``license`` or
            ``metamodel`` is not valid.
        TypeError: ``kind`` is not a ``ProjectKind``, such as its value as a
            string.
    """
    if not isinstance(kind, ProjectKind):
        raise TypeError(f"`kind` is {kind!r}, not a `ProjectKind`")

    if not Path(project_dir).exists():
        Path(project_dir).mkdir()

    sysand_rs.do_init_py_local_file(
        name, publisher, version, str(project_dir), metamodel, kind.value, license
    )


__all__ = ["ProjectKind", "init"]
