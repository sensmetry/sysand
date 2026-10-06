# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from .. import _sysand_core as sysand_rs

from pathlib import Path


def install_path(*, env_path: str | Path, iri: str, location: str | Path) -> None:
    """Install a local project into an environment.

    Args:
        env_path: The environment directory.
        iri: The IRI to install the project under.
        location: The project to copy: a KPAR file or a project directory.

    Raises:
        ValueError: ``iri`` is not an IRI (``publisher/name`` is not one).
        ProjectError: there is no readable, complete project at
            ``location``.
        EnvError: the environment is missing, unreadable or malformed, the
            project cannot be written into it, or the environment already
            has this version as an editable or workspace project.
    """
    sysand_rs.do_env_install_path_py(str(env_path), iri, str(location))


__all__ = ["install_path"]
