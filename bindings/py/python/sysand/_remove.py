# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing

from . import _sysand_core as sysand_rs

from ._identify import check_named
from ._model import InterchangeProjectUsage

from pathlib import Path


@typing.overload
def remove(
    *, project_dir: Path | str, iri: str
) -> typing.List[InterchangeProjectUsage]: ...


@typing.overload
def remove(
    *, project_dir: Path | str, publisher: str, name: str
) -> typing.List[InterchangeProjectUsage]: ...


def remove(
    *,
    project_dir: Path | str,
    iri: str | None = None,
    publisher: str | None = None,
    name: str | None = None,
) -> typing.List[InterchangeProjectUsage]:
    """Remove a dependency from the project in ``project_dir``.

    The dependency is named one of two ways:

    ``iri``
        The resource usage of that IRI, taken as given.

    ``publisher`` and ``name``
        Every usage of that project, of any kind: index, directory and KPAR
        usages, and a ``pkg:sysand`` resource usage. Spell both exactly as
        the usage does, or fully normalized (``acme-labs``/``my-lib``).

    When no usage matches, but the project is declared anyway (as a usage
    that ``iri`` does not name, or spelled differently), that raises
    :class:`ProjectError` saying how it is declared, rather than reporting it
    missing.

    Args:
        project_dir: The project directory, the one holding ``.project.json``.
        iri: The dependency's IRI.
        publisher: The dependency's publisher, given together with ``name``.
        name: The dependency's name, given together with ``publisher``.

    Returns:
        The usages that were removed, in declaration order, in the shape
        :func:`info_path` returns them. Normally one: sysand never adds the
        same project twice, but it tolerates a manifest that declares it
        more than once.

    Raises:
        TypeError: neither form was given, both were, or only one of
            ``publisher`` and ``name`` was.
        ProjectError: the project is missing or malformed, ``iri`` is not an
            IRI, ``publisher`` or ``name`` is not valid, no usage
            of the dependency is declared, or it is declared only otherwise.
    """
    check_named("remove", iri, publisher, name)
    return sysand_rs.do_remove_py(str(project_dir), iri, publisher, name)  # type: ignore


__all__ = ["remove"]
