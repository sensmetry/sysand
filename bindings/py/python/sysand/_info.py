# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from __future__ import annotations

from ._model import InterchangeProjectInfo, InterchangeProjectMetadata

import sysand._sysand_core as sysand_rs  # type: ignore

import typing
from pathlib import Path
from os import getcwd


def info_path(
    path: str | Path = ".",
) -> typing.Tuple[InterchangeProjectInfo, InterchangeProjectMetadata] | None:
    return sysand_rs.do_info_py_path(str(path))  # type: ignore


def info(
    uri: str,
    *,
    relative_file_root: str | Path | None = None,
    index_urls: str | typing.List[str] | None = None,
) -> typing.List[typing.Tuple[InterchangeProjectInfo, InterchangeProjectMetadata]]:
    if relative_file_root is None:
        relative_file_root = getcwd()

    if isinstance(index_urls, str):
        index_urls = [index_urls]

    return sysand_rs.do_info_py(uri, relative_file_root, index_urls)  # type: ignore


__all__ = [
    "info_path",
    "info",
]
