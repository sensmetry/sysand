#!/usr/bin/env python3

# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""Prepend the SPDX license and copyright header to files that lack one.

Files that already contain an `SPDX-License-Identifier` line near the top are
left alone. Exits with status 1 if any file was modified or has an unsupported
type, so pre-commit reports it.
"""

import datetime
import re
import sys
from pathlib import Path

LICENSE = "MIT OR Apache-2.0"
COPYRIGHT = "Sysand contributors <opensource@sensmetry.com>"

# Number of leading lines searched for an existing header.
HEADER_SEARCH_LINES = 10

# PEP 263 source encoding declaration, valid only on the first two lines.
CODING_COOKIE = re.compile(r"^[ \t\f]*#.*?coding[:=][ \t]*[-\w.]+")

LINE_COMMENT_PREFIX = {
    ".rs": "//",
    ".java": "//",
    ".groovy": "//",
    ".js": "//",
    ".cjs": "//",
    ".mjs": "//",
    ".py": "#",
    ".sh": "#",
}


def header_lines(suffix: str) -> list[str] | None:
    body = [
        f"SPDX-License-Identifier: {LICENSE}",
        f"SPDX-FileCopyrightText: © {datetime.date.today().year} {COPYRIGHT}",
    ]
    if suffix == ".xml":
        return ["<!--", *body, "-->"]
    prefix = LINE_COMMENT_PREFIX.get(suffix)
    if prefix is None:
        return None
    return [f"{prefix} {line}" for line in body]


def preamble_length(lines: list[str], suffix: str) -> int:
    """Return the number of leading lines that must stay above the header."""
    keep = 0
    if lines and (lines[0].startswith("#!") or lines[0].startswith("<?xml")):
        keep = 1
    # An encoding cookie may be on line 1, or on line 2 after a shebang.
    if suffix == ".py" and keep < len(lines) and CODING_COOKIE.match(lines[keep]):
        keep += 1
    return keep


def add_header(path: Path) -> bool | None:
    """Return whether the file was modified, or `None` if its type is unsupported."""
    header = header_lines(path.suffix)
    if header is None:
        return None

    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    if any("SPDX-License-Identifier" in line for line in lines[:HEADER_SEARCH_LINES]):
        return False

    keep = preamble_length(lines, path.suffix)

    block = [*lines[:keep]]
    if keep:
        block.append("\n")
    block.extend(line + "\n" for line in header)
    rest = lines[keep:]
    if rest and rest[0].strip():
        block.append("\n")

    path.write_text("".join(block + rest), encoding="utf-8")
    return True


def main(argv: list[str]) -> int:
    failed = False
    for name in argv:
        result = add_header(Path(name))
        if result is None:
            print(f"{name}: unsupported file type", file=sys.stderr)
            failed = True
        elif result:
            print(f"added SPDX header to {name}")
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
