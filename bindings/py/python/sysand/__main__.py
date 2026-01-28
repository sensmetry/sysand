# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

import sys

from sysand._sysand_core import _run_cli  # type: ignore


def main() -> int:
    is_success = _run_cli(["sysand"] + sys.argv[1:])
    return 0 if is_success else 1


if __name__ == "__main__":
    sys.exit(main())
