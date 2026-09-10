# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

from ._auth import AuthPolicy, Resolution
from ._model import InterchangeProjectInfo, InterchangeProjectMetadata

import sysand._sysand_core as sysand_rs  # type: ignore

import typing
from pathlib import Path


def info_path(
    path: str | Path = ".",
) -> typing.Tuple[InterchangeProjectInfo, InterchangeProjectMetadata]:
    return sysand_rs.do_info_py_path(str(path))  # type: ignore


def info(
    uri: str,
    *,
    index_urls: str | typing.List[str] | None = None,
    resolution: Resolution | None = None,
    auth: AuthPolicy | None = None,
) -> typing.Tuple[InterchangeProjectInfo, InterchangeProjectMetadata]:
    """Fetch the best-matching version's ``.project.json`` and ``.meta.json``
    for ``uri``.

    Without ``resolution`` and ``index_urls`` no index is consulted at all
    (only local files and URLs). ``index_urls`` is the older way to name
    exactly the indexes to use; ``resolution`` is the general one, shared
    with every other call that reaches an index. Passing both is an error.
    ``auth`` defaults to :meth:`AuthPolicy.none`.

    Raises:
        NotFoundError: no source knows ``uri``.
        AuthError: an index refused the request (HTTP 401/403).
        IndexProtocolError: an index answered with something malformed.
        ResolutionError: any other resolution failure.
    """
    if isinstance(index_urls, str):
        index_urls = [index_urls]
    if index_urls is not None:
        if resolution is not None:
            raise ValueError("pass either index_urls or resolution, not both")
        resolution = Resolution(default_index=index_urls, use_config=False)

    return sysand_rs.do_info_py(  # type: ignore
        uri,
        resolution._spec() if resolution is not None else None,
        auth._spec() if auth is not None else None,
    )


__all__ = [
    "info_path",
    "info",
]
