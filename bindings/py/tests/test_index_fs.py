# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`sysand.versions()` against a filesystem index."""

from __future__ import annotations

import json
import zipfile
from pathlib import Path

import pytest
from mockindex import run_cli_in

import sysand

DEP = "pkg:sysand/mock/dep"


def write_kpar(path: Path, publisher: str | None, name: str, version: str) -> None:
    info = {"name": name, "version": version}
    if publisher is not None:
        info["publisher"] = publisher
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_STORED) as archive:
        archive.writestr(".project.json", json.dumps(info))
        archive.writestr(".meta.json", '{"index":{},"created":"2026-01-01T00:00:00Z"}')


@pytest.fixture
def denied_network(monkeypatch: pytest.MonkeyPatch) -> None:
    for name in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy"):
        monkeypatch.setenv(name, "http://127.0.0.1:9")
    monkeypatch.setenv("NO_PROXY", "")


def test_versions_from_structured_index(tmp_path: Path, denied_network: None) -> None:
    index = tmp_path / "index"
    archive = tmp_path / "dep.kpar"
    write_kpar(archive, "mock", "dep", "0.3.0")
    # `sysand index` acts on the current directory.
    index.mkdir()
    assert run_cli_in(index, "index", "init") == 0
    assert run_cli_in(index, "index", "add", "--kpar-path", str(archive)) == 0
    assert json.loads((index / "sysand-index-config.json").read_text()) == {}

    listing = sysand.versions(
        iri=DEP,
        resolution=sysand.Resolution(default_index=[str(index)], use_config=False),
    )
    assert listing["iri"] == DEP
    assert listing["versions"] == ["0.3.0"]
    assert listing["ignored"] == []

    # The `file://` spelling names the same directory.
    listing = sysand.versions(
        iri=DEP,
        resolution=sysand.Resolution(default_index=[index.as_uri()], use_config=False),
    )
    assert listing["versions"] == ["0.3.0"]


def test_directory_without_index_json_is_refused(tmp_path: Path) -> None:
    empty = tmp_path / "empty"
    empty.mkdir()
    with pytest.raises(sysand.ResolutionError, match="is not a Sysand index"):
        sysand.versions(
            iri=DEP,
            resolution=sysand.Resolution(default_index=[str(empty)], use_config=False),
        )
