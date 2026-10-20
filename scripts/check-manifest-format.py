#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>
"""Check that the tree holds no `model.toml` spellings the format refuses.

Three rules of the manifest format (`design/model-toml.md`) are ones that
a grep can catch and a test cannot always reach -- documentation, README
examples, fixture generators, binding tests in languages the Rust suite
never compiles:

1. `manifest-version` is the semver *string* `"1.0.0"`, not the integer
   `1`.
2. A remote KPAR usage's key is `kpar-url`, not `url`.
3. Every `[project]` declares a `metamodel`, or inherits one with the
   `metamodel.workspace` marker.

Rule 3 reads files named `model.toml` and the fenced `toml` blocks of
`design/*.md`. A missing `metamodel` is reported by
`ProjectManifest::resolve`, not by the parser, so an inline fixture in a
unit test that stops at parsing is a deliberate fragment rather than a
manifest. What this check is for is the manifests and the examples no
test reads.

Every exclusion below names a line that is *supposed* to contain a refused
spelling, and says why. Excluding a whole path would hide a real
regression elsewhere in the same file, so each is a `path:line` plus a
substring that must still be on that line: if the fixture moves or
changes, the exclusion stops matching and the check speaks up.

Usage:
    scripts/check-manifest-format.py            # check the tree
    scripts/check-manifest-format.py --self-test
"""

from __future__ import annotations

import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# --------------------------------------------------------------------
# the rules

# `manifest-version`, with its value captured. A negative lookahead does
# not work here: `[ \t]*` backtracks to zero width and the lookahead then
# passes on the space. The value is read instead, and a version is good
# iff it starts with a quote -- written `"1.0.0"` in a real file and
# `\"1.0.0\"` inside a Rust or Python string fixture.
MANIFEST_VERSION = re.compile(r"manifest-version[ \t]*=[ \t]*(\S*)")

# A `url` usage key. The token must be exactly `url` -- `urls.workspace`
# and `urls.website` are a different key and must not hit -- at the
# start of a TOML line, which inside a Rust or Python string fixture is
# a literal `\n` escape rather than a physical line break. The value
# must start with a quote or `{`, which is what distinguishes a TOML
# assignment from Rust (`url = server.url()`). Spaces around `=` are
# required because every TOML file here is formatted; a shell script is
# not TOML at all and is skipped.
URL_KEY = re.compile(
    r"""(?: ^ | \\n | [{,] ) [ \t]*     # a TOML line start, real or escaped
        \\?"?url\\?"?                   # the key, quoted or bare
        [ \t]+ = [ \t]+ \\?["{]         # assigned a string or inline table
    """,
    re.VERBOSE,
)

# A `[project]` table header, and the two spellings that satisfy it.
PROJECT_TABLE = re.compile(r"^\[project\][ \t]*$", re.MULTILINE)
HAS_METAMODEL = re.compile(
    # `metamodel = "..."`, or the marker in either of its forms:
    # `metamodel.workspace = true` and `metamodel.workspace = "<group>"`.
    r"^[ \t]*metamodel(\.workspace)?[ \t]*=",
    re.MULTILINE,
)

FENCED_TOML = re.compile(r"^```toml\n(.*?)^```", re.MULTILINE | re.DOTALL)

# A block that elides its own body is an excerpt, not a manifest: the
# documents use `# ...` to show one key in the table it belongs to. The
# documentation-fidelity tests of `manifest/mod_tests.rs` complete such
# a block with the required keys rather than reading it whole, and so
# does a human.
ELIDED = re.compile(r"^[ \t]*#[ \t]*\.\.\.[ \t]*$", re.MULTILINE)


@dataclass(frozen=True)
class Allowed:
    """One line that is meant to hold a refused spelling."""

    path: str
    line: int
    must_contain: str
    why: str


ALLOWED: tuple[Allowed, ...] = (
    # --- rule 1 -----------------------------------------------------
    Allowed(
        "core/src/manifest/mod_tests.rs",
        168,
        "manifest-version = 1",
        "the fixture that proves an integer version is refused",
    ),
    Allowed(
        "core/src/manifest/mod_tests.rs",
        408,
        "manifest-version =",
        "the fixture for a `manifest-version` key with no value at all",
    ),
    # --- rule 2 -----------------------------------------------------
    Allowed(
        "core/src/manifest/usage_tests.rs",
        1021,
        "url = ",
        "the fixture that proves a `url` usage key is an unknown key",
    ),
)

# The `url` *crate*. A dependency table is not a manifest, and these
# declare it.
CARGO_MANIFESTS = frozenset(
    {
        "core/Cargo.toml",
        "sysand/Cargo.toml",
        "bindings/py/Cargo.toml",
        "bindings/js/Cargo.toml",
        "bindings/java/Cargo.toml",
        "Cargo.toml",
    }
)

# Generated, and not ours to spell.
SKIP_SUFFIXES = (".lock", "-lock.json", "package-lock.json")
SKIP_DIRS = ("target/", ".git/", "stdlib_assets/")


# This file. Its whole content is the spellings it searches for -- the
# patterns, the exclusion table, the self-test fixtures -- so it cannot
# be checked against them, and it holds no manifest to check. Skipped by
# path rather than by `ALLOWED` entries, whose line numbers would go
# stale on the first edit to this file. The checker reads `git ls-files`,
# so the script only sees itself once it is tracked.
SELF = Path(__file__).resolve().relative_to(ROOT).as_posix()


def tracked_files() -> list[Path]:
    out = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    paths = []
    for name in out.split("\0"):
        if not name or name == SELF or name.endswith(SKIP_SUFFIXES):
            continue
        if any(name.startswith(d) or f"/{d}" in name for d in SKIP_DIRS):
            continue
        paths.append(Path(name))
    return paths


def allowed_at(path: str, line_no: int, text: str) -> bool:
    return any(
        a.path == path and a.line == line_no and a.must_contain in text for a in ALLOWED
    )


def check_line_rules(path: str, text: str) -> list[str]:
    """Rules 1 and 2, which are about a single line wherever it appears."""
    problems = []
    is_cargo = path in CARGO_MANIFESTS
    previous = ""
    for line_no, line in enumerate(text.splitlines(), start=1):
        version = MANIFEST_VERSION.search(line)
        good = version is not None and version.group(1).lstrip("\\").startswith('"')
        if version and not good and not allowed_at(path, line_no, line):
            problems.append(
                f"{path}:{line_no}: `manifest-version` is the string "
                f'`"1.0.0"`, not an integer\n    {line.strip()}'
            )
        if is_cargo or path.endswith((".sh", ".ps1")):
            continue
        url = URL_KEY.search(line)
        if (
            url
            and not is_index_key(previous, line[: url.start()])
            and not allowed_at(path, line_no, line)
        ):
            problems.append(
                f"{path}:{line_no}: a remote KPAR usage's key is "
                f"`kpar-url`, not `url`\n    {line.strip()}"
            )
        previous = line
    return problems


def is_index_key(previous: str, before: str) -> bool:
    """Whether a `url` key belongs to an `[[index]]` table.

    `[[index]]` takes a `url`: it is the index configuration's key, not a
    usage's, and keeps that name. The table header is on the
    line before -- a physical line, or one written as a literal `\\n` in a
    string fixture, which leaves it earlier on the same line.
    """
    return previous.strip() == "[[index]]" or "[[index]]" in before


def check_project_tables(path: str, text: str, offset: int = 0) -> list[str]:
    """Rule 3, over one manifest's worth of TOML.

    `offset` is the line the text starts on, for a block lifted out of a
    Markdown document.
    """
    if ELIDED.search(text):
        return []
    header = PROJECT_TABLE.search(text)
    if not header:
        # A `[workspace]`-only `model.toml` declares no project.
        return []
    if HAS_METAMODEL.search(text):
        return []
    line_no = text[: header.start()].count("\n") + 1 + offset
    problem = (
        f"{path}:{line_no}: `[project]` declares no `metamodel` "
        f"and no `metamodel.workspace` marker"
    )
    return [problem]


def check_tree() -> list[str]:
    problems = []
    for path in tracked_files():
        full = ROOT / path
        try:
            text = full.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        name = path.as_posix()
        problems += check_line_rules(name, text)
        if path.name == "model.toml":
            problems += check_project_tables(name, text)
        elif path.parts[:1] == ("design",) and path.suffix == ".md":
            # A design document's examples are manifests that nothing
            # else compiles. `model-toml.md` and `workspaces.md` have
            # the documentation-fidelity tests of `mod_tests.rs`;
            # `architecture.md` has only this.
            for block in FENCED_TOML.finditer(text):
                at = text[: block.start(1)].count("\n")
                problems += check_project_tables(name, block.group(1), at)
    return problems


# --------------------------------------------------------------------
# self-test


def self_test() -> int:
    """Each rule catches its violation; each exclusion class passes."""
    cases: list[tuple[str, str, bool]] = [
        # (label, text, should_be_flagged) for the line rules
        ("integer manifest version", "manifest-version = 1\n", True),
        ("bare manifest version", "manifest-version =\n", True),
        ("string manifest version", 'manifest-version = "1.0.0"\n', False),
        ("escaped string version", 'manifest-version = \\"1.0.0\\"\n', False),
        ("url usage key", 'url = "https://example.com/a.kpar"\n', True),
        ("indented url key", '    url = "https://example.com/a.kpar"\n', True),
        ("escaped-newline url key", 'x\\nurl = \\"https://e.com/a.kpar\\"\n', True),
        ("kpar-url key", 'kpar-url = "https://example.com/a.kpar"\n', False),
        ("urls marker", "urls.workspace = true\n", False),
        ("urls table key", 'urls.website = "https://example.com"\n', False),
        ("shell assignment", 'url="$1"\n', False),
        ("rust call", "let url = server.url();\n", False),
        ("index url key", '[[index]]\nurl = "https://example.com"\n', False),
        ("escaped index url key", '[[index]]\\nurl = \\"https://e.com\\"\n', False),
        (
            "usage url after an index",
            '[[index]]\nurl = "a"\n[[inline-usages]]\nurl = "b"\n',
            True,
        ),
    ]
    failures = []
    for label, text, want in cases:
        got = bool(check_line_rules("x.toml", text))
        if got != want:
            failures.append(
                f"line rules: {label!r}: expected "
                f"{'a hit' if want else 'no hit'}, got the opposite"
            )

    tables: list[tuple[str, str, bool]] = [
        ("project without metamodel", '[project]\nname = "a"\n', True),
        ("project with metamodel", '[project]\nmetamodel = "x"\n', False),
        ("marker, boolean", "[project]\nmetamodel.workspace = true\n", False),
        ("marker, group", '[project]\nmetamodel.workspace = "kerml"\n', False),
        ("workspace-only file", "[workspace]\nmembers = []\n", False),
        ("elided excerpt", "[project]\n# ...\n", False),
    ]
    for label, text, want in tables:
        got = bool(check_project_tables("x/model.toml", text))
        if got != want:
            failures.append(
                f"project tables: {label!r}: expected "
                f"{'a hit' if want else 'no hit'}, got the opposite"
            )

    # Every exclusion must still match the line it names: a fixture that
    # moved leaves a stale exclusion, which is how a check goes quiet.
    for a in ALLOWED:
        full = ROOT / a.path
        if not full.is_file():
            failures.append(f"exclusion: {a.path} does not exist")
            continue
        lines = full.read_text(encoding="utf-8").splitlines()
        if a.line > len(lines) or a.must_contain not in lines[a.line - 1]:
            failures.append(
                f"exclusion: {a.path}:{a.line} no longer contains "
                f"{a.must_contain!r} -- move it or drop it"
            )

    for failure in failures:
        print(f"self-test: {failure}", file=sys.stderr)
    if failures:
        return 1
    print(f"self-test: {len(cases) + len(tables) + len(ALLOWED)} cases pass")
    return 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    problems = check_tree()
    if not problems:
        return 0
    print(
        "The manifest format the design review left is not what these lines spell:\n",
        file=sys.stderr,
    )
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"\n{len(problems)} problem(s). If a line is meant to hold the "
        f"old spelling, add it to ALLOWED in {Path(__file__).name} with "
        f"the reason.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
