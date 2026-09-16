# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`sysand.migrate`, the json-to-toml conversion."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

import sysand


def _git(path: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=path, check=True, capture_output=True)


def _json_project(path: Path, publisher: str | None = "acme") -> None:
    info: dict[str, object] = {"name": "widgets", "version": "1.2.3", "usage": []}
    if publisher is not None:
        info["publisher"] = publisher
    (path / ".project.json").write_text(json.dumps(info))
    (path / ".meta.json").write_text(
        json.dumps({"created": "2001-02-03T04:05:06Z", "index": {}})
    )


def _repo(path: Path, commit: bool = True) -> None:
    _git(path, "init", "-q")
    _git(path, "config", "user.email", "test@example.com")
    _git(path, "config", "user.name", "Test")
    if commit:
        _git(path, "add", "-A")
        _git(path, "commit", "-qm", "initial")


def test_migrate_converts_a_project(tmp_path: Path) -> None:
    _json_project(tmp_path)
    _repo(tmp_path)

    sysand.migrate(tmp_path)

    assert (tmp_path / "model.toml").is_file()
    assert not (tmp_path / ".project.json").exists()
    assert not (tmp_path / ".meta.json").exists()
    assert 'name = "widgets"' in (tmp_path / "model.toml").read_text()


def test_migrate_supplies_a_missing_publisher(tmp_path: Path) -> None:
    _json_project(tmp_path, publisher=None)
    _repo(tmp_path)

    with pytest.raises(sysand.ProjectError):
        sysand.migrate(tmp_path)
    assert (tmp_path / ".project.json").is_file(), "nothing was deleted"

    sysand.migrate(tmp_path, publisher="acme")
    assert 'publisher = "acme"' in (tmp_path / "model.toml").read_text()


def test_migrate_refuses_a_dirty_tree(tmp_path: Path) -> None:
    _json_project(tmp_path)
    _repo(tmp_path)
    (tmp_path / "stray.txt").write_text("uncommitted\n")

    with pytest.raises(sysand.ProjectError):
        sysand.migrate(tmp_path)
    assert not (tmp_path / "model.toml").exists()

    sysand.migrate(tmp_path, allow_dirty=True)
    assert (tmp_path / "model.toml").is_file()


def test_migrate_refuses_a_directory_with_no_repository(tmp_path: Path) -> None:
    _json_project(tmp_path)

    sysand.migrate(tmp_path, allow_no_vcs=True)
    assert (tmp_path / "model.toml").is_file()
