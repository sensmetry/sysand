# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

import sys

from ._sysand_core import _run_cli  # type: ignore


def main() -> int:
    # Annotated rather than returned directly: `_run_cli` is untyped, and
    # `mypy --strict` rejects returning `Any` from an `int` function.
    code: int = _run_cli(["sysand"] + sys.argv[1:])
    return code


if __name__ == "__main__":
    sys.exit(main())
