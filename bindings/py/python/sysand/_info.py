# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

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
) -> typing.Tuple[InterchangeProjectInfo, InterchangeProjectMetadata]:
    if isinstance(index_urls, str):
        index_urls = [index_urls]

    return sysand_rs.do_info_py(uri, index_urls)  # type: ignore


__all__ = [
    "info_path",
    "info",
]
