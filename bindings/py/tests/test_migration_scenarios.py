# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""End-to-end checks for editing a usage constraint through the Python API.

Each test starts from `baseline` / `make_baseline` (conftest.py): a real
project whose manifest, lockfile and `.sysand` were produced by the shipped
CLI against the mock index, with the library pinned to the 0.10 line. The
tests then move that constraint to the 0.11 line with
`sysand.set_usage_constraint` and check what happened to the manifest.
"""

from __future__ import annotations

import json

import pytest

import sysand
from conftest import Baseline, MakeBaseline
from mockindex import LEGACY_CONSTRAINT, LIBRARY, TARGET_CONSTRAINT, usage


def migrate_constraint(baseline: Baseline) -> None:
    change = sysand.set_usage_constraint(baseline.root, LIBRARY, TARGET_CONSTRAINT)
    assert change["found"] is True
    assert change["changed"] is True
    assert change["old_constraint"] == LEGACY_CONSTRAINT
    assert change["new_constraint"] == TARGET_CONSTRAINT


def test_manifest_fidelity(baseline: Baseline) -> None:
    # A manifest last written by sysand changes in exactly one line; unknown
    # keys and the key order survive.
    before = baseline.manifest.read_text().splitlines(keepends=True)
    migrate_constraint(baseline)
    after = baseline.manifest.read_text().splitlines(keepends=True)

    assert len(before) == len(after)
    differing = [(b, a) for b, a in zip(before, after) if b != a]
    assert len(differing) == 1, differing
    [(old_line, new_line)] = differing
    assert LEGACY_CONSTRAINT in old_line
    assert new_line == old_line.replace(LEGACY_CONSTRAINT, TARGET_CONSTRAINT)

    manifest = json.loads("".join(after))
    assert manifest["x-consumer-extension"] == {"migrated": False}
    assert list(manifest) == [
        "version",
        "name",
        "publisher",
        "x-consumer-extension",
        "usage",
    ]
    [library_usage] = manifest["usage"]
    assert library_usage["x-consumer-note"] == "pre-0.11"


def test_add_when_missing(make_baseline: MakeBaseline) -> None:
    # The project does not declare the library at all: `set_usage_constraint`
    # reports or refuses without writing, and `add` is the way to declare it.
    baseline = make_baseline(include_library=False)
    before = baseline.manifest.read_bytes()

    change = sysand.set_usage_constraint(
        baseline.root, LIBRARY, TARGET_CONSTRAINT, must_exist=False
    )
    assert change["found"] is False
    assert change["changed"] is False
    assert baseline.manifest.read_bytes() == before

    with pytest.raises(sysand.ProjectError) as excinfo:
        sysand.set_usage_constraint(baseline.root, LIBRARY, TARGET_CONSTRAINT)
    assert excinfo.value.wrote is False

    assert sysand.add(baseline.root, LIBRARY, TARGET_CONSTRAINT) is True
    manifest = json.loads(baseline.manifest.read_text())
    assert manifest["usage"] == [usage(LIBRARY, TARGET_CONSTRAINT)]
    # A second add merges into the existing usage rather than adding one.
    assert sysand.add(baseline.root, LIBRARY, TARGET_CONSTRAINT) is False
    assert len(json.loads(baseline.manifest.read_text())["usage"]) == 1
