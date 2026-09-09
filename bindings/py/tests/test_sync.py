# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`sysand.sync()` against the mock index."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import sysand
from mockindex import MockIndex, usage

DEP = "pkg:sysand/mock/dep"
OTHER = "pkg:sysand/mock/other"


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


def changed(entries: list[sysand.SyncedProject]) -> set[tuple[str, str]]:
    return {(e["iri"], e["version"]) for e in entries}


def test_sync_installs_then_keeps(tmp_path: Path, mock_index: MockIndex) -> None:
    mock_index.publish(DEP, "1.0.0", files={"dep.sysml": b"package Dep;"})
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0")])
    res = resolution(mock_index)
    env_dir = root / sysand.env.DEFAULT_ENV_NAME
    sysand.lock(root, resolution=res)

    outcome = sysand.sync(root, resolution=res)

    [installed] = outcome["installed"]
    assert (installed["iri"], installed["version"]) == (DEP, "1.0.0")
    assert installed["path"] is not None
    assert (env_dir / installed["path"] / "dep.sysml").is_file()
    assert outcome["pruned"] == [] and outcome["kept"] == []
    assert {
        p["identifiers"][0]: p["version"]
        for p in sysand.env.projects(env_dir)
        if p["identifiers"]
    } == {DEP: "1.0.0"}

    again = sysand.sync(root, resolution=res)
    assert again["installed"] == [] and again["pruned"] == []
    assert again["kept"] == [installed]


def test_sync_prunes_unless_no_prune(tmp_path: Path, mock_index: MockIndex) -> None:
    mock_index.publish(DEP, "1.0.0")
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0")])
    res = resolution(mock_index)
    env_dir = root / sysand.env.DEFAULT_ENV_NAME
    sysand.lock(root, resolution=res)
    sysand.sync(root, resolution=res)

    extra = tmp_path / "extra"
    extra.mkdir()
    sysand.init("extra", "acme", "0.1.0", extra)
    sysand.env.install_path(env_dir, "urn:kpar:extra", extra)

    kept = sysand.sync(root, resolution=res, no_prune=True)
    assert kept["pruned"] == []
    assert any(
        "urn:kpar:extra" in p["identifiers"] for p in sysand.env.projects(env_dir)
    )

    pruned = sysand.sync(root, resolution=res)
    assert pruned["pruned"] == [
        {"iri": "urn:kpar:extra", "version": "0.1.0", "path": None}
    ]
    assert changed(pruned["kept"]) == {(DEP, "1.0.0")}
    assert not any(
        "urn:kpar:extra" in p["identifiers"] for p in sysand.env.projects(env_dir)
    )


def test_sync_requires_a_lockfile(tmp_path: Path, mock_index: MockIndex) -> None:
    mock_index.publish(DEP, "1.0.0")
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0")])
    res = resolution(mock_index)

    with pytest.raises(sysand.ProjectError) as excinfo:
        sysand.sync(root, resolution=res)
    assert excinfo.value.wrote is False
    assert "lock" in str(excinfo.value)
    assert not (root / sysand.env.DEFAULT_ENV_NAME).exists(), (
        "nothing is created without a lockfile"
    )

    result = sysand.lock(root, resolution=res, write=False)
    assert not (root / "sysand-lock.toml").exists()
    outcome = sysand.sync(root, lock=result, resolution=res)
    assert changed(outcome["installed"]) == {(DEP, "1.0.0")}
    outcome = sysand.sync(root, lock=result["text"], resolution=res)
    assert changed(outcome["kept"]) == {(DEP, "1.0.0")}


def test_sync_half_synced(tmp_path: Path, mock_index: MockIndex) -> None:
    mock_index.publish(DEP, "1.0.0")
    mock_index.publish(OTHER, "2.0.0")
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0"), usage(OTHER, ">=2.0.0")])
    res = resolution(mock_index)
    result = sysand.lock(root, resolution=res)
    # Installs happen in lockfile order (by name): `dep` then `other`.
    assert [p["name"] for p in result["projects"] if p["name"] != "proj"] == [
        "dep",
        "other",
    ]
    mock_index.fail_next(MockIndex.kpar_path(OTHER, "2.0.0"), 500)

    with pytest.raises(sysand.SyncError) as excinfo:
        sysand.sync(root, resolution=res)

    error = excinfo.value
    assert error.wrote is True
    assert error.partial is not None
    assert changed(error.partial["installed"]) == {(DEP, "1.0.0")}
    assert error.partial["pruned"] == []
    assert "500" in str(error)


def test_sync_auth(tmp_path: Path, mock_index: MockIndex) -> None:
    mock_index.publish(DEP, "1.0.0")
    mock_index.require_bearer("s3cret")
    root = project_with(tmp_path, [usage(DEP, ">=1.0.0")])
    res = resolution(mock_index)
    bearer = sysand.AuthPolicy.bearer(mock_index.url + "**", "s3cret")
    sysand.lock(root, resolution=res, auth=bearer)

    with pytest.raises(sysand.SyncError) as excinfo:
        sysand.sync(root, resolution=res)
    assert excinfo.value.wrote is False
    assert excinfo.value.partial is not None
    assert excinfo.value.partial["installed"] == []

    outcome = sysand.sync(root, resolution=res, auth=bearer)
    assert changed(outcome["installed"]) == {(DEP, "1.0.0")}
