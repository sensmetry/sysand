# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`sysand.sync(verify=True)` and `repair=True`."""

from __future__ import annotations

import hashlib
import json
import shutil
from pathlib import Path

import sysand


def write_dep(root: Path) -> None:
    root.mkdir(parents=True)
    model = "package dep { }\n"
    (root / "dep.sysml").write_text(model)
    (root / ".project.json").write_text(
        json.dumps({"name": "dep", "publisher": "acme", "version": "1.0.0"})
    )
    (root / ".meta.json").write_text(
        json.dumps(
            {
                "index": {"dep": "dep.sysml"},
                "created": "2026-01-01T00:00:00Z",
                "checksum": {
                    "dep.sysml": {
                        "algorithm": "SHA256",
                        "value": hashlib.sha256(model.encode()).hexdigest(),
                    }
                },
            }
        )
    )


def synced_app(tmp_path: Path) -> Path:
    app = tmp_path / "app"
    app.mkdir()
    # A directory usage: `lock` resolves it without an index and records
    # the dependency's canonical digest, which the tests then verify.
    (app / ".project.json").write_text(
        json.dumps(
            {
                "name": "app",
                "publisher": "acme",
                "version": "0.1.0",
                "usage": [{"dir": "deps/dep", "publisher": "acme", "name": "dep"}],
            }
        )
    )
    (app / ".meta.json").write_text('{"index":{},"created":"2026-01-01T00:00:00Z"}')
    write_dep(app / "deps" / "dep")
    sysand.lock(path=str(app), resolution=sysand.Resolution(use_index=False))
    sysand.sync(path=str(app), resolution=sysand.Resolution(use_index=False))
    return app


def installed_dep(app: Path) -> Path:
    outcome = sysand.sync(
        path=str(app), resolution=sysand.Resolution(use_index=False), verify=True
    )
    [entry] = outcome["verified"]
    assert entry["path"] is not None
    return app / ".sysand" / entry["path"]


def test_verify_clean(tmp_path: Path) -> None:
    app = synced_app(tmp_path)
    outcome = sysand.sync(
        path=str(app), resolution=sysand.Resolution(use_index=False), verify=True
    )
    assert [e["version"] for e in outcome["verified"]] == ["1.0.0"]
    assert outcome["mismatched"] == []
    assert outcome["unrecorded"] == []
    assert outcome["stale"] == []
    assert outcome["repaired"] == []
    assert outcome["installed"] == []


def test_verify_reports_tampered_file_and_repair_heals(tmp_path: Path) -> None:
    app = synced_app(tmp_path)
    tampered = installed_dep(app) / "dep.sysml"
    tampered.write_text("package dep { part p; }\n")

    outcome = sysand.sync(
        path=str(app), resolution=sysand.Resolution(use_index=False), verify=True
    )
    [mismatch] = outcome["mismatched"]
    assert mismatch["version"] == "1.0.0"
    assert mismatch["kind"] == "checksum"
    assert mismatch["file"] == "dep.sysml"
    assert mismatch["message"].startswith("installed project `")
    assert "does not match the lockfile checksum" in mismatch["message"]
    # Verifying wrote nothing.
    assert tampered.read_text() == "package dep { part p; }\n"

    outcome = sysand.sync(
        path=str(app), resolution=sysand.Resolution(use_index=False), repair=True
    )
    assert [e["version"] for e in outcome["repaired"]] == ["1.0.0"]
    assert outcome["mismatched"] == []
    assert tampered.read_text() == "package dep { }\n"

    outcome = sysand.sync(
        path=str(app), resolution=sysand.Resolution(use_index=False), verify=True
    )
    assert outcome["mismatched"] == []
    assert len(outcome["verified"]) == 1


def test_verify_reports_environment_gone(tmp_path: Path) -> None:
    app = synced_app(tmp_path)
    shutil.rmtree(app / ".sysand")
    outcome = sysand.sync(
        path=str(app), resolution=sysand.Resolution(use_index=False), verify=True
    )
    [mismatch] = outcome["mismatched"]
    assert mismatch["kind"] == "not-recorded"
    assert not (app / ".sysand").exists()


def test_stale_lockfile_is_reported_not_repaired(tmp_path: Path) -> None:
    app = synced_app(tmp_path)
    # The manifest drops its usage; the lockfile still records it.
    manifest = json.loads((app / ".project.json").read_text())
    manifest["usage"] = []
    (app / ".project.json").write_text(json.dumps(manifest))
    lock_before = (app / "sysand-lock.toml").read_text()

    outcome = sysand.sync(
        path=str(app), resolution=sysand.Resolution(use_index=False), repair=True
    )
    [stale] = outcome["stale"]
    assert stale["kind"] == "removed"
    assert stale["manifest"] == str(app / ".project.json")
    assert "is in `sysand-lock.toml` and no longer in" in stale["message"]
    assert (app / "sysand-lock.toml").read_text() == lock_before
