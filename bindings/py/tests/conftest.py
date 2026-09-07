# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import dataclasses
import json
import os
import typing
from pathlib import Path

import pytest
from pytest_httpserver import HTTPServer

import sysand
from mockindex import (
    LEGACY_CONSTRAINT,
    LIBRARY,
    MockIndex,
    run_cli_in,
    usage,
)


@pytest.fixture(scope="session", autouse=True)
def _claim_logger(tmp_path_factory: pytest.TempPathFactory) -> None:
    # Whoever installs the global `log` logger first owns it for the process:
    # binding functions install `pyo3_log` (records reach `caplog`), while
    # `_run_cli` tries `env_logger`. Make one cheap binding call before any
    # test so `_run_cli` can never steal the logger from the tests that
    # assert on log records; `run_cli` then only prints a "failed to set up
    # logger" warning and continues.
    sysand.env.env(tmp_path_factory.mktemp("logger") / sysand.env.DEFAULT_ENV_NAME)


def isolate_sysand_env(monkeypatch: pytest.MonkeyPatch) -> None:
    """Mirrors `sysand/tests/common/mod.rs`.

    Drop every ambient `SYSAND_*` override (index, config, credentials, and
    whatever is added later) so tests see only what they set themselves, then
    point the credential store at the debug-only seam in
    `sysand/src/credential_store.rs` so the developer's real OS keyring is
    never touched.
    """
    for var in [v for v in os.environ if v.startswith("SYSAND_")]:
        monkeypatch.delenv(var)
    monkeypatch.setenv("SYSAND_TEST_CREDENTIAL_STORE", ":absent:")
    monkeypatch.setenv("NO_COLOR", "1")


@pytest.fixture(autouse=True)
def _sysand_isolation(monkeypatch: pytest.MonkeyPatch) -> None:
    isolate_sysand_env(monkeypatch)


@pytest.fixture
def mock_index(httpserver: HTTPServer) -> typing.Iterator[MockIndex]:
    index = MockIndex(httpserver)
    yield index
    assert httpserver.handler_errors == [], (
        f"mock index handler raised: {httpserver.handler_errors}"
    )
    assert index.requests("/index.json") == [], (
        "`/index.json` must never be fetched: lookups go through the "
        "per-project `versions.json` (see `sysand/tests/cli_lock.rs`)"
    )


@dataclasses.dataclass
class Baseline:
    """A project whose manifest, lockfile and `.sysand` are at library 0.10.3."""

    root: Path
    index: MockIndex

    @property
    def manifest(self) -> Path:
        return self.root / ".project.json"

    @property
    def lockfile(self) -> Path:
        return self.root / "sysand-lock.toml"

    @property
    def env_dir(self) -> Path:
        return self.root / sysand.env.DEFAULT_ENV_NAME

    def installed(self) -> list[str]:
        """Names of the version-stamped install directories."""
        lib = self.env_dir / "lib"
        return sorted(p.name for p in lib.iterdir()) if lib.is_dir() else []

    def snapshot(self) -> dict[str, object]:
        """Everything a migration may touch, for "unchanged" assertions."""
        return {
            "manifest": self.manifest.read_bytes(),
            "lockfile": self.lockfile.read_bytes(),
            "env.toml": (self.env_dir / "env.toml").read_bytes(),
            "installed": self.installed(),
        }

    def cli(self, *args: str) -> bool:
        return run_cli_in(
            self.root, *args, "--no-config", "--default-index", self.index.url
        )


MakeBaseline = typing.Callable[..., Baseline]


@pytest.fixture
def make_baseline(tmp_path: Path, mock_index: MockIndex) -> MakeBaseline:
    """Factory for the project's real starting state.

    The manifest is hand-written on purpose: it carries an unknown top-level
    key, an unknown key inside the usage, and non-canonical key order and
    whitespace (the input the manifest-fidelity scenario needs), which the
    typed `add` path would not preserve. The lockfile and environment are then
    produced by the shipped CLI, in-process, so the 0.10.3 state is real rather
    than hand-written and depends on none of the APIs the scenarios exercise.
    """

    def make(
        *,
        include_library: bool = True,
        extra_usages: typing.Sequence[dict] = (),
        library_files: typing.Mapping[str, bytes] = {
            "library.sysml": b"package Library;\n"
        },
    ) -> Baseline:
        mock_index.publish(LIBRARY, "0.10.3", files=library_files)

        root = tmp_path / "migrating"
        root.mkdir()
        sysand.init("migrating", "acme", "1.0.0", root)

        usages: list[dict] = []
        if include_library:
            legacy = usage(LIBRARY, LEGACY_CONSTRAINT)
            legacy["x-consumer-note"] = "pre-0.11"
            usages.append(legacy)
        usages.extend(extra_usages)
        manifest = {
            "version": "1.0.0",
            "name": "migrating",
            "publisher": "acme",
            "x-consumer-extension": {"migrated": False},
            "usage": usages,
        }
        (root / ".project.json").write_text(json.dumps(manifest, indent=4) + "\n")

        baseline = Baseline(root=root, index=mock_index)
        assert baseline.cli("lock"), "baseline `sysand lock` failed"
        assert baseline.cli("sync"), "baseline `sysand sync` failed"
        return baseline

    return make


@pytest.fixture
def baseline(make_baseline: MakeBaseline) -> Baseline:
    return make_baseline()
