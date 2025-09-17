# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from __future__ import annotations

import sysand._sysand_core as sysand_rs  # type: ignore
from sysand._sysand_core import DEFAULT_ENV_NAME

from ._install import (
    install_path,
)

from pathlib import Path


def env(path: str | Path = DEFAULT_ENV_NAME) -> None:
    sysand_rs.do_env_py_local_dir(str(path))


__all__ = [
    "env",
    "DEFAULT_ENV_NAME",
    ## Install
    "install_path",
]
