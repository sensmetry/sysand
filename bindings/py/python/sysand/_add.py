# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing

from . import _sysand_core as sysand_rs

from ._identify import check_named

from pathlib import Path


@typing.overload
def add(
    *,
    project_dir: Path | str,
    iri: str,
    version_constraint: str,
) -> bool: ...


@typing.overload
def add(
    *,
    project_dir: Path | str,
    publisher: str,
    name: str,
    version_constraint: str,
) -> bool: ...


def add(
    *,
    project_dir: Path | str,
    iri: str | None = None,
    publisher: str | None = None,
    name: str | None = None,
    version_constraint: str,
) -> bool:
    """Declare a dependency of the project in ``project_dir``.

    The dependency is named one of two ways, and exactly one of them must be
    given:

    ``iri``
        The project's IRI, taken as given: ``publisher/name`` is not an IRI
        and is refused. This declares a resource usage, the untyped kind that
        KerML specifies, even for a ``pkg:sysand`` IRI.

    ``publisher`` and ``name``
        The project with that publisher and name, resolved from the index.
        This declares an index usage, which :func:`info_path` returns as
        :class:`InterchangeProjectUsageIndex`. Spell both exactly as the
        project does: locking fails otherwise.

    Either way, ``version_constraint`` is a semver requirement such as
    ``">=1.0.0"``, and is required: pass ``"*"`` to accept any version.

    Directory and KPAR usages cannot be added yet.

    Args:
        project_dir: The project directory, the one holding ``.project.json``.
        iri: The dependency's IRI.
        publisher: The dependency's publisher, given together with ``name``.
        name: The dependency's name, given together with ``publisher``.
        version_constraint: A semver requirement such as ``">=1.0.0"``.

    Returns:
        ``True`` when a new usage was added, ``False`` when the project was
        already declared and the call merged into (or ignored for) the
        existing usage.

    Raises:
        TypeError: neither form was given, both were, only one of
            ``publisher`` and ``name`` was, or ``version_constraint`` is
            missing or ``None``.
        ProjectError: the project is missing or malformed, ``iri`` is not an
            IRI, ``publisher`` or ``name`` is not valid,
            ``version_constraint`` is not a semver requirement, or the same
            project is already declared by a usage of another kind (which
            would declare it twice, from two different sources) or by an
            index usage spelled differently.
    """
    # Every usage `add` writes gets a constraint, which an index usage
    # requires.
    if version_constraint is None:
        raise TypeError(
            'add() takes a `version_constraint`; pass "*" to accept any version'
        )
    check_named("add", iri, publisher, name)
    return sysand_rs.do_add_py(  # type: ignore
        str(project_dir), iri, publisher, name, version_constraint
    )


__all__ = ["add"]
