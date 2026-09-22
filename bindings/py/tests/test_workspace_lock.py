# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""The advisory lock from the binding: a mutating
call fails fast with `LockHeldError` while another process holds
`.sysand/.lock`; the binding never waits."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

import sysand

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="fcntl.flock")


def app(tmp_path: Path) -> Path:
    root = tmp_path / "app"
    root.mkdir()
    (root / ".project.json").write_text(
        json.dumps({"name": "app", "publisher": "acme", "version": "0.1.0"})
    )
    (root / ".meta.json").write_text('{"index":{},"created":"2026-01-01T00:00:00Z"}')
    return root


def hold(root: Path, shared: bool = False):
    import fcntl

    (root / ".sysand").mkdir(exist_ok=True)
    handle = open(root / ".sysand" / ".lock", "a+")
    fcntl.flock(handle, (fcntl.LOCK_SH if shared else fcntl.LOCK_EX) | fcntl.LOCK_NB)
    return handle


def test_mutating_calls_fail_fast(tmp_path: Path) -> None:
    root = app(tmp_path)
    resolution = sysand.Resolution(use_index=False)
    sysand.lock(path=str(root), resolution=resolution)
    holder = hold(root)
    try:
        with pytest.raises(sysand.LockHeldError, match="another process is using"):
            sysand.lock(path=str(root), resolution=resolution)
        with pytest.raises(sysand.LockHeldError):
            sysand.sync(path=str(root), resolution=resolution)
        with pytest.raises(sysand.LockHeldError):
            sysand.sync(path=str(root), resolution=resolution, verify=True)
        with pytest.raises(sysand.LockHeldError):
            sysand.env.env(path=root / ".sysand")
        assert not (root / ".sysand" / "env.toml").exists()
        # Reading does not take the lock.
        assert sysand.info_path(project_dir=str(root))[0]["name"] == "app"
        # Neither does a solve that does not write.
        sysand.lock(path=str(root), resolution=resolution, write=False)
    finally:
        holder.close()
    sysand.sync(path=str(root), resolution=resolution)
    assert (root / ".sysand" / "env.toml").is_file()


def test_verify_shares_the_lock(tmp_path: Path) -> None:
    root = app(tmp_path)
    resolution = sysand.Resolution(use_index=False)
    sysand.lock(path=str(root), resolution=resolution)
    sysand.sync(path=str(root), resolution=resolution)
    holder = hold(root, shared=True)
    try:
        outcome = sysand.sync(path=str(root), resolution=resolution, verify=True)
        assert outcome["mismatched"] == []
        with pytest.raises(sysand.LockHeldError):
            sysand.sync(path=str(root), resolution=resolution)
    finally:
        holder.close()
