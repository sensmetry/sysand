# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`add` against a mock index: like `sysand add`, it locks and syncs unless
told not to."""

from __future__ import annotations

import json
import logging
from pathlib import Path

import pytest

import sysand
from mockindex import MockIndex

DEP = "pkg:sysand/acme-labs/my-lib"


def resolution(index: MockIndex) -> sysand.Resolution:
    return sysand.Resolution(default_index=[index.url], use_config=False)


def publish_dep(index: MockIndex, version: str = "1.0.0") -> None:
    index.publish(DEP, version, publisher="Acme Labs", name="My Lib")


@pytest.fixture(autouse=True)
def _log_everything(caplog: pytest.LogCaptureFixture) -> None:
    # `pyo3_log` caches a logger's level on its first use, and these tests are
    # the first to use some of the loggers that later tests (`test_basic_init`,
    # `test_cli_logger`) assert records from. Cached at DEBUG, the records
    # still reach Python, whose own levels filter them as usual.
    caplog.set_level(logging.DEBUG)


def init(tmp_path: Path) -> Path:
    root = tmp_path / "app"
    sysand.init(project_dir=root, name="app", publisher="acme", version="1.0.0")
    return root


def usages(root: Path) -> list[dict]:
    return json.loads((root / ".project.json").read_text()).get("usage", [])


def installed(root: Path) -> list[str]:
    env_path = root / sysand.env.DEFAULT_ENV_NAME
    # The project itself is there too, as an editable entry
    return [
        p["identifiers"][0]
        for p in sysand.env.projects(env_path=env_path)
        if not p["editable"]
    ]


def test_add_locks_and_syncs(tmp_path: Path, mock_index: MockIndex) -> None:
    publish_dep(mock_index)
    root = init(tmp_path)

    assert (
        sysand.add(
            project_dir=root,
            publisher="Acme Labs",
            name="My Lib",
            version_constraint="^1",
            resolution=resolution(mock_index),
        )
        is True
    )

    assert usages(root) == [
        {"publisher": "Acme Labs", "name": "My Lib", "versionConstraint": "^1"}
    ]
    assert 'name = "My Lib"' in (root / "sysand-lock.toml").read_text()
    assert installed(root) == [DEP]


def test_add_no_sync_only_locks(tmp_path: Path, mock_index: MockIndex) -> None:
    publish_dep(mock_index)
    root = init(tmp_path)

    sysand.add(
        project_dir=root,
        publisher="Acme Labs",
        name="My Lib",
        version_constraint="^1",
        no_sync=True,
        resolution=resolution(mock_index),
    )

    assert (root / "sysand-lock.toml").is_file()
    assert not (root / sysand.env.DEFAULT_ENV_NAME).exists()


def test_add_normalized_takes_the_locked_spelling(
    tmp_path: Path, mock_index: MockIndex
) -> None:
    publish_dep(mock_index)
    root = init(tmp_path)

    sysand.add(
        project_dir=root,
        publisher="acme-labs",
        name="my-lib",
        version_constraint="^1",
        resolution=resolution(mock_index),
    )

    assert usages(root) == [
        {"publisher": "Acme Labs", "name": "My Lib", "versionConstraint": "^1"}
    ]


def test_add_misspelled_fails_and_restores_the_manifest(
    tmp_path: Path, mock_index: MockIndex
) -> None:
    publish_dep(mock_index)
    root = init(tmp_path)
    before = (root / ".project.json").read_text()

    with pytest.raises(
        sysand.ProjectError, match="spell the usage exactly as `Acme Labs/My Lib`"
    ):
        sysand.add(
            project_dir=root,
            publisher="Acme labs",
            name="My Lib",
            version_constraint="^1",
            resolution=resolution(mock_index),
        )

    assert (root / ".project.json").read_text() == before


def test_add_unsatisfiable_is_a_solve_error(
    tmp_path: Path, mock_index: MockIndex
) -> None:
    publish_dep(mock_index)
    root = init(tmp_path)
    before = (root / ".project.json").read_text()

    with pytest.raises(sysand.SolveError):
        sysand.add(
            project_dir=root,
            publisher="Acme Labs",
            name="My Lib",
            version_constraint="^2",
            resolution=resolution(mock_index),
        )

    assert (root / ".project.json").read_text() == before


def test_add_no_lock_checks_against_the_environment(
    tmp_path: Path, mock_index: MockIndex
) -> None:
    publish_dep(mock_index)
    root = init(tmp_path)

    # Nothing is installed to check the spelling against, and the index is
    # not asked
    with pytest.raises(
        sysand.ProjectError,
        match="no version matching `\\^1` is installed in the local environment",
    ):
        sysand.add(
            project_dir=root,
            publisher="acme-labs",
            name="my-lib",
            version_constraint="^1",
            no_lock=True,
            resolution=resolution(mock_index),
        )
    assert mock_index.requests() == []
    assert usages(root) == []

    # Once installed (by locking and syncing it through another usage), the
    # normalized spelling is recovered from the environment
    sysand.add(
        project_dir=root,
        publisher="Acme Labs",
        name="My Lib",
        version_constraint="^1",
        resolution=resolution(mock_index),
    )
    sysand.remove(project_dir=root, publisher="Acme Labs", name="My Lib")
    assert installed(root) == [DEP]
    requests = len(mock_index.requests())

    sysand.add(
        project_dir=root,
        publisher="acme-labs",
        name="my-lib",
        version_constraint="^1",
        no_lock=True,
        resolution=resolution(mock_index),
    )
    assert usages(root) == [
        {"publisher": "Acme Labs", "name": "My Lib", "versionConstraint": "^1"}
    ]
    assert len(mock_index.requests()) == requests
