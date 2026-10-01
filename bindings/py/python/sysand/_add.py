# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing

from . import _sysand_core as sysand_rs

from ._auth import AuthPolicy, Resolution

from pathlib import Path


@typing.overload
def add(
    *,
    project_dir: Path | str,
    iri: str,
    version_constraint: str,
    lock: bool = True,
    sync: bool = True,
    prune: bool = True,
    resolution: Resolution | None = None,
    auth: AuthPolicy | None = None,
) -> bool: ...


@typing.overload
def add(
    *,
    project_dir: Path | str,
    publisher: str,
    name: str,
    version_constraint: str,
    lock: bool = True,
    sync: bool = True,
    prune: bool = True,
    resolution: Resolution | None = None,
    auth: AuthPolicy | None = None,
) -> bool: ...


def add(
    *,
    project_dir: Path | str,
    iri: str | None = None,
    publisher: str | None = None,
    name: str | None = None,
    version_constraint: str,
    lock: bool = True,
    sync: bool = True,
    prune: bool = True,
    resolution: Resolution | None = None,
    auth: AuthPolicy | None = None,
) -> bool:
    """Declare a dependency of the project in ``project_dir``, then lock and
    sync, as ``sysand add`` does.

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
        project does, or fully normalized (``acme-labs``/``my-lib``) to take
        the project's own spelling. With ``lock=False``, the spelling is
        checked against, or taken from, the versions installed in the
        project's environment, and the call fails when none that
        ``version_constraint`` accepts is installed.

    Either way, ``version_constraint`` is a semver requirement such as
    ``">=1.0.0"``, and is required: pass ``"*"`` to accept any version.

    Directory and KPAR usages cannot be added yet.

    Then, if ``lock``, the project's dependencies are locked into
    ``sysand-lock.toml`` and, if ``sync`` too, installed into its
    environment. If either fails, ``.project.json`` is restored.
    ``resolution`` defaults to :class:`Resolution` ``()`` (the CLI's
    semantics); ``auth`` defaults to :meth:`AuthPolicy.none`.

    Args:
        project_dir: The project directory, the one holding ``.project.json``.
        iri: The dependency's IRI.
        publisher: The dependency's publisher, given together with ``name``.
        name: The dependency's name, given together with ``publisher``.
        version_constraint: A semver requirement such as ``">=1.0.0"``.
        lock: Lock after editing ``.project.json``; when ``False``, only
            edit it, and do not sync either.
        sync: Install the locked projects into the environment.
        prune: When syncing, remove projects the lockfile no longer lists.
        resolution: Where to look for dependencies.
        auth: How to authenticate to indexes.

    Returns:
        ``True`` when a new usage was added, ``False`` when the project was
        already declared and the call merged into (or ignored for) the
        existing usage.

    Raises:
        TypeError: neither form was given, both were, only one of
            ``publisher`` and ``name`` was, or ``version_constraint`` is
            missing or ``None``.
        SolveError: no compatible set of versions exists.
        ProjectError: the project is missing or malformed, ``iri`` is not an
            IRI, ``publisher`` or ``name`` is not valid or not spelled as the
            project does, ``version_constraint`` is not a semver requirement,
            the same project is already declared by a usage of another kind
            (which would declare it twice, from two different sources) or by
            an index usage spelled differently, or syncing failed.
        AuthError, IndexProtocolError, ResolutionError: an index could not be
            used.
    """
    # Every usage `add` writes gets a constraint, which an index usage
    # requires.
    if version_constraint is None:
        raise TypeError(
            'add() takes a `version_constraint`; pass "*" to accept any version'
        )
    if resolution is None:
        resolution = Resolution()
    return sysand_rs.do_add_py(  # type: ignore
        str(project_dir),
        iri,
        publisher,
        name,
        version_constraint,
        not lock,
        not sync,
        not prune,
        resolution._spec(),
        auth._spec() if auth is not None else None,
    )


__all__ = ["add"]
