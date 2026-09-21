# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing

from pathlib import Path

from . import _sysand_core as sysand_rs

from ._errors import ProjectError
from ._identify import project_iri
from ._model import UsageConstraintChange


@typing.overload
def set_usage_constraint(
    *,
    project_dir: str | Path,
    iri: str,
    version_constraint: str,
    must_exist: bool = True,
) -> UsageConstraintChange: ...


@typing.overload
def set_usage_constraint(
    *,
    project_dir: str | Path,
    publisher: str,
    name: str,
    version_constraint: str,
    must_exist: bool = True,
) -> UsageConstraintChange: ...


def set_usage_constraint(
    *,
    project_dir: str | Path,
    iri: str | None = None,
    publisher: str | None = None,
    name: str | None = None,
    version_constraint: str,
    must_exist: bool = True,
) -> UsageConstraintChange:
    """Set the version constraint of a dependency of the project in
    ``project_dir``.

    The dependency is named as :func:`add` names it: by ``iri``, taken as
    given, or by ``publisher`` and ``name``. A constraint can be replaced but
    not removed; pass ``"*"`` to accept any version.

    Only that one ``versionConstraint`` value is touched: every other key,
    including keys sysand does not know, and the document's key order are
    preserved. Whitespace is sysand's own pretty format, so a manifest last
    written by sysand changes in exactly one line. An unchanged constraint
    does not touch the file at all.

    Args:
        project_dir: The project directory, the one holding ``.project.json``.
        iri: The dependency's IRI.
        publisher: The dependency's publisher, given together with ``name``.
        name: The dependency's name, given together with ``publisher``.
        version_constraint: A semver requirement such as ``">=0.11.0, <0.12.0"``.
        must_exist: Raise :class:`ProjectError` when no usage matches. With
            ``False`` a missing usage is reported as ``found=False`` instead;
            a missing or unreadable ``.project.json`` still raises. It does
            not apply to a usage that is declared but cannot hold a
            constraint: that one is not missing.

    Returns:
        Whether the usage was found and whether its constraint changed, with
        the constraint before and after the call.

    Raises:
        TypeError: neither form was given, both were, only one of
            ``publisher`` and ``name`` was, or ``version_constraint`` is ``None``.
        ProjectError: ``version_constraint`` is not a valid semver requirement,
            ``iri`` is not an IRI, ``publisher`` or ``name`` is not
            valid, the manifest is missing or malformed, the dependency is
            declared more than once, is declared as a kind that carries no
            version constraint, or (with ``must_exist``) is not declared at
            all. ``wrote`` is always ``False``.
    """
    # A constraint cannot be cleared, so that a usage `add` gave a
    # constraint keeps one.
    if version_constraint is None:
        raise TypeError(
            'set_usage_constraint() cannot clear a constraint; pass "*" to accept any version'
        )
    resolved = project_iri("set_usage_constraint", iri, publisher, name)
    found, changed, old, new = sysand_rs.do_set_usage_constraint_py(
        str(project_dir), resolved, version_constraint
    )
    change = UsageConstraintChange(
        found=found,
        changed=changed,
        old_version_constraint=old,
        new_version_constraint=new,
    )
    if must_exist and not found:
        raise ProjectError(
            f"usage `{resolved}` is not declared in `{Path(project_dir) / '.project.json'}`",
            wrote=False,
        )
    return change


__all__ = ["set_usage_constraint"]
