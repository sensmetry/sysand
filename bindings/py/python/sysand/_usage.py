# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from pathlib import Path

from . import _sysand_core as sysand_rs

from ._errors import ProjectError
from ._model import UsageConstraintChange


def set_usage_constraint(
    *,
    path: str | Path,
    identifier: str,
    constraint: str | None,
    must_exist: bool = True,
) -> UsageConstraintChange:
    """Set (or, with ``None``, clear) the version constraint of the usage
    identified by ``identifier`` in the project at ``path``.

    Only that one ``versionConstraint`` value is touched: every other key,
    including keys sysand does not know, and the document's key order are
    preserved. Whitespace is sysand's own pretty format, so a manifest last
    written by sysand changes in exactly one line. An unchanged constraint
    does not touch the file at all.

    ``identifier`` may be an IRI or the ``publisher/name`` shorthand, matched
    the way ``add`` matches.

    Args:
        path: The project directory.
        identifier: The project's IRI or ``publisher/name`` shorthand.
        constraint: A semver requirement such as ``">=0.11.0, <0.12.0"``, or
            ``None`` to remove the constraint.
        must_exist: Raise :class:`ProjectError` when no usage matches. With
            ``False`` a missing usage is reported as ``found=False`` instead;
            a missing or unreadable ``.project.json`` still raises. It does
            not apply to a usage that is declared but cannot hold a
            constraint: that one is not missing.

    Raises:
        ProjectError: ``constraint`` is not a valid semver requirement,
            ``identifier`` is a malformed shorthand, the manifest is missing
            or malformed, ``identifier`` is declared more than once, is
            declared as a kind that carries no version constraint, or (with
            ``must_exist``) is not declared at all. ``wrote`` is always
            ``False``.
    """
    matched, found, changed, old, new = sysand_rs.do_set_usage_constraint_py(
        str(path), identifier, constraint
    )
    change = UsageConstraintChange(
        identifier=matched,
        found=found,
        changed=changed,
        old_constraint=old,
        new_constraint=new,
    )
    if must_exist and not found:
        raise ProjectError(
            f"usage `{matched}` is not declared in `{Path(path) / '.project.json'}`",
            wrote=False,
        )
    return change


__all__ = ["set_usage_constraint"]
