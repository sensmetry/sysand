# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

import sys

import sysand._sysand_core as sysand_rs  # type: ignore


def main() -> int:
    is_success = sysand_rs._run_cli(["sysand"] + sys.argv[1:])
    return 0 if is_success else 1


if __name__ == "__main__":
    sys.exit(main())
