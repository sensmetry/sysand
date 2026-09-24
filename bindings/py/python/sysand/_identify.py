# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing


def check_named(
    function: str,
    iri: typing.Optional[str],
    publisher: typing.Optional[str],
    name: typing.Optional[str],
) -> None:
    """Check that ``function`` was asked to name a usage one way: by ``iri``,
    or by ``publisher`` and ``name``. Whether the values themselves are valid
    is checked by the extension, the same way for every function.

    Raises:
        TypeError: neither ``iri`` nor ``publisher`` and ``name`` were given,
            both were, or only one of ``publisher`` and ``name`` was.
    """
    if (publisher is None) != (name is None):
        raise TypeError(f"{function}() takes `publisher` and `name` together")
    if (iri is None) == (publisher is None):
        raise TypeError(
            f"{function}() takes exactly one of `iri` and `publisher` with `name`"
        )
