# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from pathlib import Path

import sysand._sysand_core as sysand_rs  # type: ignore

from ._errors import ProjectError
from ._model import UsageConstraintChange


def set_usage_constraint(
    path: str | Path,
    resource: str,
    constraint: str | None,
    *,
    must_exist: bool = True,
) -> UsageConstraintChange:
    """Set (or, with ``None``, clear) the version constraint of the usage
    naming ``resource`` in the project at ``path``.

    Only that one ``versionConstraint`` value is touched: every other key,
    including keys sysand does not know, and the document's key order are
    preserved. Whitespace is sysand's own pretty format, so a manifest last
    written by sysand changes in exactly one line. An unchanged constraint
    does not touch the file at all.

    ``resource`` may be an IRI or the ``publisher/name`` shorthand, matched
    the way ``add`` matches.

    Args:
        path: The project directory.
        resource: The usage's resource IRI or shorthand.
        constraint: A semver requirement such as ``">=0.11.0, <0.12.0"``, or
            ``None`` to remove the constraint.
        must_exist: Raise :class:`ProjectError` when no usage matches. With
            ``False`` a missing usage is reported as ``found=False`` instead;
            a missing or unreadable ``.project.json`` still raises.

    Raises:
        ValueError: ``constraint`` is not a valid semver requirement, or
            ``resource`` is a malformed shorthand.
        ProjectError: the manifest is missing or malformed, ``resource`` is
            declared more than once, or (with ``must_exist``) not at all.
            ``wrote`` is always ``False``.
    """
    matched, found, changed, old, new = sysand_rs.do_set_usage_constraint_py(
        str(path), resource, constraint
    )
    change = UsageConstraintChange(
        resource=matched,
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
