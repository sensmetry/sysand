# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing

from . import _sysand_core as sysand_rs


def project_iri(
    function: str,
    iri: typing.Optional[str],
    publisher: typing.Optional[str],
    name: typing.Optional[str],
) -> str:
    """The IRI that ``function`` was asked to name a project by: ``iri`` as
    given, or the ``pkg:sysand`` PURL of ``publisher`` and ``name``.

    ``publisher`` and ``name`` may be unnormalized (``"Acme Labs"``); the
    PURL holds them normalized (``acme-labs``). Whether ``iri`` is a valid
    IRI is checked by the extension, the same way for every function.

    Raises:
        TypeError: neither ``iri`` nor ``publisher`` and ``name`` were given,
            both were, or only one of ``publisher`` and ``name`` was.
        ProjectError: ``publisher`` or ``name`` is not valid, even
            unnormalized.
    """
    if (publisher is None) != (name is None):
        raise TypeError(f"{function}() takes `publisher` and `name` together")
    if (iri is None) == (publisher is None):
        raise TypeError(
            f"{function}() takes exactly one of `iri` and `publisher` with `name`"
        )
    if iri is not None:
        return iri
    assert publisher is not None and name is not None
    return sysand_rs.index_purl_py(publisher, name)  # type: ignore
