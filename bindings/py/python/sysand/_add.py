# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing

from . import _sysand_core as sysand_rs

from ._model import InterchangeProjectUsage

from pathlib import Path


@typing.overload
def add(
    *,
    path: Path | str,
    iri: str,
    version_constraint: str | None = None,
) -> bool: ...


@typing.overload
def add(*, path: Path | str, usage: InterchangeProjectUsage) -> bool: ...


def add(
    *,
    path: Path | str,
    iri: str | None = None,
    version_constraint: str | None = None,
    usage: InterchangeProjectUsage | None = None,
) -> bool:
    """Declare a dependency of the project at ``path``.

    The dependency is named one of two ways, and exactly one of them must be
    given:

    ``iri``
        An IRI, or the ``publisher/name`` shorthand, which is expanded to
        ``pkg:sysand/<publisher>/<name>``. This declares a resource usage,
        the untyped kind, and is the only form that takes
        ``version_constraint``.

    ``usage``
        The usage itself, in the model's own shape. Nothing is guessed at or
        expanded: a resource usage here needs a full IRI. This reaches every
        usage kind -- directory, KPAR path, and any kind added after this
        release -- without a new function.

    Raises:
        TypeError: neither ``iri`` nor ``usage`` was given, both were, or
            ``version_constraint`` was combined with ``usage``.
        ProjectError: the project is missing or malformed, ``iri`` is a
            malformed shorthand, the usage is malformed, or the same project
            is already declared by a usage of a different kind (which would
            declare it twice, from two different sources).

    Returns:
        ``True`` when a new usage was added, ``False`` when the project was
        already declared and the call merged into (or ignored for) the
        existing usage.
    """
    if (iri is None) == (usage is None):
        raise TypeError("add() takes exactly one of `iri` and `usage`")

    if usage is not None:
        if version_constraint is not None:
            raise TypeError(
                "add() takes `version_constraint` with `iri`; a usage carries its own"
            )
        # The extension takes the model's full shape; the TypedDict lets a
        # resource usage leave `version_constraint` out.
        complete: typing.Mapping[str, object] = (
            {"version_constraint": None, **usage} if "resource" in usage else usage
        )
        return sysand_rs.do_add_usage_py(str(path), complete)  # type: ignore

    return sysand_rs.do_add_py(str(path), iri, version_constraint)  # type: ignore


__all__ = ["add"]
