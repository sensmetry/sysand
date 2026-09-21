# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing

from . import _sysand_core as sysand_rs

from ._model import InterchangeProjectUsage

from pathlib import Path


def remove(*, path: Path | str, iri: str) -> typing.List[InterchangeProjectUsage]:
    """Remove the resource usage of ``iri`` (an IRI or ``publisher/name``
    shorthand) from the project at ``path``.

    Only resource usages are removed. A directory or KPAR-path usage of the
    same project is *not* removed, and is not reported as missing either:
    that raises :class:`ProjectError`, since the project is declared, just
    not as a resource usage.

    Raises:
        ProjectError: the project is missing or malformed, ``iri`` is a
            malformed shorthand, no usage of it is declared, or it is
            declared only as a usage of another kind.

    Returns:
        The usages that were removed, in declaration order. Normally one:
        sysand never adds the same resource twice, but it tolerates a
        manifest that declares it more than once.
    """
    return sysand_rs.do_remove_py(str(path), iri)  # type: ignore


__all__ = ["remove"]
