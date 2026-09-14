# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""End-to-end checks for editing a usage constraint through the Python API.

Each test starts from `baseline` / `make_baseline` (conftest.py): a real
project whose manifest, lockfile and `.sysand` were produced by the shipped
CLI against the mock index, with the library pinned to the 0.10 line. The
tests then move that constraint to the 0.11 line with
`sysand.set_usage_constraint`, resolve the result with `sysand.lock`, install
it with `sysand.sync` and check the environment with `sysand.env.projects`;
or they check what the tool driving the migration can find out about the
project before it starts.
"""

from __future__ import annotations

import json
import os
import typing

import pytest

import sysand
from conftest import Baseline, MakeBaseline
from mockindex import (
    DEPENDENT,
    LEGACY_CONSTRAINT,
    LIBRARY,
    TARGET_CONSTRAINT,
    MockIndex,
    usage,
)


def resolution(index: MockIndex) -> sysand.Resolution:
    return sysand.Resolution(default_index=[index.url], use_config=False)


def project_version(projects: typing.Sequence[dict], iri: str) -> str | None:
    for project in projects:
        if iri in project["identifiers"]:
            return project["version"]
    return None


def env_versions(baseline: Baseline) -> dict[str, str]:
    return {
        project["identifiers"][0]: project["version"]
        for project in sysand.env.projects(baseline.env_dir)
        if project["identifiers"]
    }


def changed(entries: typing.Sequence[dict]) -> set[tuple[str, str]]:
    return {(e["iri"], e["version"]) for e in entries}


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


def test_workspace_detection(baseline: Baseline) -> None:
    # A tool that must not edit a project inside a workspace checks
    # `discover` first: the project root is found, the workspace root only
    # once a `.workspace.json` appears above the project.
    discovery = sysand.discover(baseline.root)
    assert discovery["workspace_root"] is None
    assert discovery["project_root"] is not None
    assert os.path.samefile(discovery["project_root"], baseline.root)

    workspace = baseline.root.parent
    (workspace / ".workspace.json").write_text('{"projects": []}\n')
    discovery = sysand.discover(baseline.root)
    assert discovery["workspace_root"] is not None
    assert os.path.samefile(discovery["workspace_root"], workspace)
    assert os.path.samefile(discovery["project_root"], baseline.root)

    nested = baseline.root / "src" / "deep"
    nested.mkdir(parents=True)
    assert os.path.samefile(sysand.discover(nested)["project_root"], baseline.root)
    assert sysand.discover(workspace)["project_root"] is None


# ---------------------------------------------------------------------------
# Resolving the new constraint


def _dependent_baseline(
    make_baseline: MakeBaseline, mock_index: MockIndex, dependent_constraint: str
) -> Baseline:
    """A baseline that also uses a second project, which itself uses the
    library under ``dependent_constraint``."""
    mock_index.publish(
        DEPENDENT,
        "1.0.0",
        usage=[usage(LIBRARY, dependent_constraint)],
        files={"dep.sysml": b"package Dep; // 1.0.0\n"},
    )
    return make_baseline(extra_usages=[usage(DEPENDENT, ">=1.0.0, <2.0.0")])


def test_dependent_excludes_new_version(
    make_baseline: MakeBaseline, mock_index: MockIndex
) -> None:
    # `"0.10.1"` is caret: `>=0.10.1, <0.11.0`. The baseline resolves; the
    # migration cannot, and the failure names the dependent as the reason.
    mock_index.publish(DEPENDENT, "1.0.0", usage=[usage(LIBRARY, "0.10.1")])
    baseline = make_baseline(extra_usages=[usage(DEPENDENT, "1.0.0")])
    mock_index.publish(LIBRARY, "0.11.0")
    res = resolution(mock_index)

    migrate_constraint(baseline)
    before = baseline.snapshot()

    with pytest.raises(sysand.SolveError) as excinfo:
        sysand.lock(baseline.root, resolution=res, write=False)
    error = excinfo.value
    assert error.wrote is False
    assert isinstance(error.report, str) and error.report
    assert any(
        c["kind"] == "Constraint"
        and c["required_by"] == DEPENDENT
        and c["iri"] == LIBRARY
        for c in error.conflicts
    ), error.conflicts

    after = baseline.snapshot()
    assert after["lockfile"] == before["lockfile"]
    assert after["env.toml"] == before["env.toml"]
    assert after["installed"] == before["installed"]


def test_no_compatible_dependent_release(
    make_baseline: MakeBaseline, mock_index: MockIndex
) -> None:
    baseline = _dependent_baseline(make_baseline, mock_index, "0.10.1")
    mock_index.publish(LIBRARY, "0.11.0")
    res = resolution(mock_index)

    migrate_constraint(baseline)
    before = baseline.snapshot()
    with pytest.raises(sysand.SolveError) as excinfo:
        sysand.lock(baseline.root, resolution=res, write=False)
    assert excinfo.value.wrote is False
    assert any(
        c["kind"] == "Constraint" and c["required_by"] == DEPENDENT
        for c in excinfo.value.conflicts
    ), excinfo.value.conflicts
    assert baseline.snapshot() == before


def test_new_version_not_published_yet(baseline: Baseline) -> None:
    index = baseline.index
    res = resolution(index)

    listing = sysand.versions(LIBRARY, resolution=res)
    assert listing["versions"] == ["0.10.3"]

    migrate_constraint(baseline)
    before = baseline.snapshot()
    with pytest.raises(sysand.SolveError) as excinfo:
        sysand.lock(baseline.root, resolution=res, write=False)
    error = excinfo.value
    assert error.wrote is False
    [conflict] = [c for c in error.conflicts if c["kind"] == "NoVersions"]
    assert conflict["iri"] == LIBRARY
    assert conflict["constraint"] == TARGET_CONSTRAINT
    assert conflict["found"] == ["0.10.3"]
    assert baseline.snapshot() == before

    # Once the version appears, the same call resolves; a dry run still
    # writes nothing.
    index.publish(LIBRARY, "0.11.0")
    result = sysand.lock(baseline.root, resolution=res, write=False)
    assert project_version(result["projects"], LIBRARY) == "0.11.0"
    assert baseline.snapshot() == before


# ---------------------------------------------------------------------------
# Installing the new version


def test_root_pins_library(baseline: Baseline) -> None:
    index = baseline.index
    index.publish(
        LIBRARY, "0.11.0", files={"library.sysml": b"package Library; // 0.11\n"}
    )
    res = resolution(index)

    migrate_constraint(baseline)

    # What is available, before anything is decided.
    listing = sysand.versions(LIBRARY, resolution=res)
    assert listing["versions"] == ["0.11.0", "0.10.3"]
    assert listing["ignored"] == []

    # The resolution, shown to the user before anything is written.
    lock_before = baseline.lockfile.read_bytes()
    dry = sysand.lock(baseline.root, resolution=res, write=False)
    assert baseline.lockfile.read_bytes() == lock_before
    assert project_version(dry["projects"], LIBRARY) == "0.11.0"

    written = sysand.lock(baseline.root, resolution=res)
    assert written["text"] == dry["text"]
    assert baseline.lockfile.read_text() == written["text"]

    outcome = sysand.sync(baseline.root, resolution=res)
    assert changed(outcome["installed"]) == {(LIBRARY, "0.11.0")}
    assert changed(outcome["pruned"]) == {(LIBRARY, "0.10.3")}
    assert outcome["kept"] == []

    # What the environment holds afterwards.
    assert env_versions(baseline) == {LIBRARY: "0.11.0"}
    [installed] = baseline.installed()
    assert installed.endswith("_0.11.0")
    assert (baseline.env_dir / "lib" / installed / "library.sysml").read_bytes() == (
        b"package Library; // 0.11\n"
    )


def test_dependent_pins_internally(
    make_baseline: MakeBaseline, mock_index: MockIndex
) -> None:
    baseline = _dependent_baseline(make_baseline, mock_index, "0.10.1")
    # The new library version appears together with a dependent release
    # that follows it.
    mock_index.publish(LIBRARY, "0.11.0")
    mock_index.publish(
        DEPENDENT,
        "1.1.0",
        usage=[usage(LIBRARY, TARGET_CONSTRAINT)],
        files={"dep.sysml": b"package Dep; // 1.1.0\n"},
    )
    res = resolution(mock_index)

    migrate_constraint(baseline)
    result = sysand.lock(baseline.root, resolution=res)
    assert project_version(result["projects"], LIBRARY) == "0.11.0"
    assert project_version(result["projects"], DEPENDENT) == "1.1.0"

    outcome = sysand.sync(baseline.root, resolution=res)
    assert changed(outcome["installed"]) == {
        (LIBRARY, "0.11.0"),
        (DEPENDENT, "1.1.0"),
    }
    assert changed(outcome["pruned"]) == {
        (LIBRARY, "0.10.3"),
        (DEPENDENT, "1.0.0"),
    }
    assert env_versions(baseline) == {LIBRARY: "0.11.0", DEPENDENT: "1.1.0"}


def test_open_ended_dependent(
    make_baseline: MakeBaseline, mock_index: MockIndex
) -> None:
    # The solver is right to keep dependent 1.0.0 and install 0.11.0; only
    # the caller's own check after `sync` can notice if the dependent no
    # longer works against the new library. sysand is silent here by design,
    # and `env.projects()` is what that check works from.
    baseline = _dependent_baseline(make_baseline, mock_index, ">=0.10")
    mock_index.publish(LIBRARY, "0.11.0")
    res = resolution(mock_index)

    migrate_constraint(baseline)
    result = sysand.lock(baseline.root, resolution=res)
    assert project_version(result["projects"], LIBRARY) == "0.11.0"
    assert project_version(result["projects"], DEPENDENT) == "1.0.0"

    outcome = sysand.sync(baseline.root, resolution=res)
    assert changed(outcome["installed"]) == {(LIBRARY, "0.11.0")}
    assert changed(outcome["pruned"]) == {(LIBRARY, "0.10.3")}
    assert changed(outcome["kept"]) == {(DEPENDENT, "1.0.0")}

    projects = sysand.env.projects(baseline.env_dir)
    assert env_versions(baseline) == {LIBRARY: "0.11.0", DEPENDENT: "1.0.0"}
    [dependent] = [p for p in projects if DEPENDENT in p["identifiers"]]
    assert dependent["editable"] is False
    assert (baseline.env_dir / dependent["path"] / "dep.sysml").is_file()


# ---------------------------------------------------------------------------
# Operational cases


def test_authenticated_index(
    baseline: Baseline, monkeypatch: pytest.MonkeyPatch
) -> None:
    index = baseline.index
    index.publish(LIBRARY, "0.11.0")
    index.require_bearer("s3cret")
    res = resolution(index)
    # Globs do not cross `/` (`core/src/auth.rs`); `**` matches every path.
    url_glob = index.url + "**"

    with pytest.raises(sysand.AuthError):
        sysand.versions(LIBRARY, resolution=res)
    with pytest.raises(sysand.AuthError):
        sysand.versions(LIBRARY, resolution=res, auth=sysand.AuthPolicy.none())

    bearer = sysand.AuthPolicy.bearer(url_glob, "s3cret")
    assert "s3cret" not in repr(bearer)
    assert "s3cret" not in str(bearer)
    listing = sysand.versions(LIBRARY, resolution=res, auth=bearer)
    assert listing["versions"] == ["0.11.0", "0.10.3"]

    # The CLI's own resolution, env vars only (`sysand/src/cred_env.rs`).
    monkeypatch.setenv("SYSAND_CRED_TEST", url_glob)
    monkeypatch.setenv("SYSAND_CRED_TEST_BEARER_TOKEN", "s3cret")
    from_env = sysand.AuthPolicy.from_env(keyring=False)
    assert "s3cret" not in repr(from_env)
    listing = sysand.versions(LIBRARY, resolution=res, auth=from_env)
    assert listing["versions"] == ["0.11.0", "0.10.3"]

    migrate_constraint(baseline)
    with pytest.raises(sysand.AuthError) as excinfo:
        sysand.lock(baseline.root, resolution=res, write=False)
    assert excinfo.value.wrote is False
    sysand.lock(baseline.root, resolution=res, auth=bearer)
    outcome = sysand.sync(baseline.root, resolution=res, auth=bearer)
    assert changed(outcome["installed"]) == {(LIBRARY, "0.11.0")}


def test_half_synced(make_baseline: MakeBaseline, mock_index: MockIndex) -> None:
    baseline = _dependent_baseline(make_baseline, mock_index, ">=0.10")
    mock_index.publish(LIBRARY, "0.11.0")
    mock_index.publish(DEPENDENT, "1.1.0", usage=[usage(LIBRARY, TARGET_CONSTRAINT)])
    res = resolution(mock_index)

    migrate_constraint(baseline)
    result = sysand.lock(baseline.root, resolution=res)
    iri_by_name = {"library": LIBRARY, "dependent": DEPENDENT}
    # `sync` installs in lockfile order; fail the archive of the entry that
    # comes last so at least one install has already landed.
    to_install = [p for p in result["projects"] if p["name"] in iri_by_name]
    assert len(to_install) == 2
    last = to_install[-1]
    last_iri = iri_by_name[last["name"]]
    first_iri = iri_by_name[to_install[0]["name"]]
    mock_index.fail_next(MockIndex.kpar_path(last_iri, last["version"]), 500)

    with pytest.raises(sysand.SyncError) as excinfo:
        sysand.sync(baseline.root, resolution=res)
    error = excinfo.value
    assert error.wrote is True
    assert changed(error.partial["installed"]) == {
        (first_iri, to_install[0]["version"])
    }
    assert (last_iri, last["version"]) not in changed(error.partial["installed"])
    assert error.partial["pruned"] == [], "a failed sync must not have pruned"
    # The 0.10.3 state is still installed alongside the partial result, so a
    # caller that kept a copy of `.sysand` can restore it by file copy.
    assert any(n.endswith("_0.10.3") for n in baseline.installed())
