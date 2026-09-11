# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`sysand.lock()` against the mock index."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import sysand
from mockindex import DEPENDENT, LIBRARY, MockIndex, usage

DEP = "pkg:sysand/mock/dep"


def resolution(index: MockIndex) -> sysand.Resolution:
    return sysand.Resolution(default_index=[index.url], use_config=False)


def project_with(tmp_path: Path, usages: list[dict]) -> Path:
    root = tmp_path / "proj"
    root.mkdir()
    sysand.init("proj", "acme", "1.0.0", root)
    manifest = json.loads((root / ".project.json").read_text())
    manifest["usage"] = usages
    (root / ".project.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return root


def by_name(result: sysand.LockResult, name: str) -> sysand.LockedProject:
    [project] = [p for p in result["projects"] if p["name"] == name]
    return project


def test_lock_write_false_leaves_no_lockfile(
    tmp_path: Path, mock_index: MockIndex
) -> None:
    mock_index.publish(DEP, "1.0.0", files={"dep.sysml": b"package Dep;"})
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0")])

    result = sysand.lock(root, resolution=resolution(mock_index), write=False)

    assert not (root / "sysand-lock.toml").exists()
    assert "lock_version = " in result["text"]
    assert {p["name"] for p in result["projects"]} == {"proj", "dep"}
    dep = by_name(result, "dep")
    assert dep["version"] == "1.0.0"
    assert dep["identifiers"] == [DEP]
    assert dep["publisher"] == "mock"
    assert any(
        mock_index.kpar_digest(DEP, "1.0.0").removeprefix("sha256:") in s
        for s in dep["sources"]
    )
    assert by_name(result, "proj")["usages"] == [DEP]


def test_lock_writes_exactly_text(tmp_path: Path, mock_index: MockIndex) -> None:
    mock_index.publish(DEP, "1.0.0")
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0")])
    res = resolution(mock_index)

    dry = sysand.lock(root, resolution=res, write=False)
    written = sysand.lock(root, resolution=res)

    lockfile = root / "sysand-lock.toml"
    assert lockfile.read_text() == written["text"] == dry["text"]
    # The lockfile is exactly what the CLI writes.
    assert lockfile.read_text() == sysand.lock(root, resolution=res)["text"]
    assert lockfile.read_text() == written["text"]


def test_lock_from_a_subdirectory_targets_the_project_root(
    tmp_path: Path, mock_index: MockIndex
) -> None:
    mock_index.publish(DEP, "1.0.0")
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0")])
    nested = root / "src" / "deep"
    nested.mkdir(parents=True)

    sysand.lock(nested, resolution=resolution(mock_index))

    assert (root / "sysand-lock.toml").is_file()
    assert not (nested / "sysand-lock.toml").exists()


def test_lock_conflicts_name_the_dependent(
    tmp_path: Path, mock_index: MockIndex
) -> None:
    mock_index.publish(LIBRARY, "0.10.3")
    mock_index.publish(LIBRARY, "0.11.0")
    mock_index.publish(DEPENDENT, "1.0.0", usage=[usage(LIBRARY, "0.10.1")])
    root = project_with(
        tmp_path,
        [usage(DEPENDENT, ">=1.0.0, <2.0.0"), usage(LIBRARY, ">=0.11.0, <0.12.0")],
    )

    with pytest.raises(sysand.SolveError) as excinfo:
        sysand.lock(root, resolution=resolution(mock_index))
    error = excinfo.value

    assert error.wrote is False
    assert error.kind == "no_solution"
    assert error.report and error.report in str(error)
    assert {
        "kind": "Constraint",
        "iri": LIBRARY,
        "constraint": "^0.10.1",
        "required_by": DEPENDENT,
    } in error.conflicts
    assert {
        "kind": "Constraint",
        "iri": LIBRARY,
        "constraint": ">=0.11.0, <0.12.0",
        "required_by": None,
    } in error.conflicts
    assert not (root / "sysand-lock.toml").exists()


def test_lock_no_matching_version(tmp_path: Path, mock_index: MockIndex) -> None:
    mock_index.publish(LIBRARY, "0.10.3")
    root = project_with(tmp_path, [usage(LIBRARY, ">=0.11.0, <0.12.0")])

    with pytest.raises(sysand.SolveError) as excinfo:
        sysand.lock(root, resolution=resolution(mock_index), write=False)
    error = excinfo.value

    assert error.kind == "retrieval"
    assert error.conflicts == [
        {
            "kind": "NoVersions",
            "iri": LIBRARY,
            "constraint": ">=0.11.0, <0.12.0",
            "found": ["0.10.3"],
            "required_by": None,
        }
    ]
    assert "requested version constraint" in str(error)


def test_lock_unknown_project_is_a_not_found_conflict(
    tmp_path: Path, mock_index: MockIndex
) -> None:
    root = project_with(tmp_path, [usage("pkg:sysand/mock/absent")])

    with pytest.raises(sysand.SolveError) as excinfo:
        sysand.lock(root, resolution=resolution(mock_index), write=False)

    [conflict] = excinfo.value.conflicts
    assert conflict["kind"] == "NotFound"
    assert conflict["iri"] == "pkg:sysand/mock/absent"


def test_lock_not_in_project(tmp_path: Path, mock_index: MockIndex) -> None:
    with pytest.raises(sysand.ProjectError) as excinfo:
        sysand.lock(tmp_path, resolution=resolution(mock_index))
    assert excinfo.value.wrote is False
    assert "not inside a project" in str(excinfo.value)


def test_lock_provided_project_is_not_fetched(
    tmp_path: Path, mock_index: MockIndex
) -> None:
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0")])
    provided: sysand.ProvidedProject = {
        "iri": DEP,
        "info": {
            "name": "dep",
            "publisher": "mock",
            "description": None,
            "version": "1.2.0",
            "license": None,
            "maintainer": [],
            "website": None,
            "topic": [],
            "usage": [],
        },
        "meta": {
            "index": {},
            "created": "2026-01-01T00:00:00Z",
            "metamodel": None,
            "includes_derived": None,
            "includes_implied": None,
            "checksum": None,
        },
    }

    result = sysand.lock(
        root, resolution=resolution(mock_index), provided=[provided], write=False
    )

    dep = by_name(result, "dep")
    assert dep["version"] == "1.2.0"
    assert dep["sources"] == [], "a provided project has nothing to install"
    assert mock_index.requests("*/versions.json") == []


def test_lock_auth(tmp_path: Path, mock_index: MockIndex) -> None:
    mock_index.publish(DEP, "1.0.0")
    mock_index.require_bearer("s3cret")
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0")])
    res = resolution(mock_index)

    with pytest.raises(sysand.AuthError) as excinfo:
        sysand.lock(root, resolution=res, write=False)
    assert excinfo.value.wrote is False

    result = sysand.lock(
        root,
        resolution=res,
        auth=sysand.AuthPolicy.bearer(mock_index.url + "**", "s3cret"),
        write=False,
    )
    assert by_name(result, "dep")["version"] == "1.0.0"
