# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from ._auth import AuthPolicy, Resolution
from ._model import InterchangeProjectInfo, InterchangeProjectMetadata

from . import _sysand_core as sysand_rs

import typing
from pathlib import Path


def info_path(
    *,
    project_dir: str | Path = ".",
) -> typing.Tuple[InterchangeProjectInfo, InterchangeProjectMetadata]:
    """Read the ``.project.json`` and ``.meta.json`` of the project in
    ``project_dir``.

    Args:
        project_dir: The project directory, the one holding ``.project.json``
            and ``.meta.json``. Defaults to the current directory. It is not
            searched upwards; see :func:`root` for that.

    Returns:
        The project's information and metadata.
    """
    return sysand_rs.do_info_py_path(str(project_dir))  # type: ignore


def info(
    *,
    iri: str,
    resolution: Resolution | None = None,
    auth: AuthPolicy | None = None,
) -> typing.Tuple[InterchangeProjectInfo, InterchangeProjectMetadata]:
    """Fetch the best-matching version's ``.project.json`` and ``.meta.json``
    for ``iri``.

    Without ``resolution`` no index is consulted at all (only local files
    and URLs). ``auth`` defaults to :meth:`AuthPolicy.none`.

    Args:
        iri: The project's IRI.
        resolution: Where to look for the project.
        auth: How to authenticate to indexes.

    Returns:
        The best-matching version's information and metadata.

    Raises:
        NotFoundError: no source knows ``iri``.
        AuthError: an index refused the request (HTTP 401/403).
        IndexProtocolError: an index answered with something malformed.
        ResolutionError: any other resolution failure.
    """
    return sysand_rs.do_info_py(  # type: ignore
        iri,
        resolution._spec() if resolution is not None else None,
        auth._spec() if auth is not None else None,
    )


__all__ = [
    "info_path",
    "info",
]
