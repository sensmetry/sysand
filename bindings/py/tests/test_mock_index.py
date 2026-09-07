# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""The `mock_index` fixture's own tests: the index layout it serves, and the
shipped CLI driving `lock` + `sync` against it (the Python twin of
`sysand/tests/cli_lock.rs::lock_and_sync_against_mock_index`)."""

from __future__ import annotations

import hashlib
import json
import os
import urllib.error
import urllib.request

import pytest

from conftest import Baseline, MakeBaseline, isolate_sysand_env
from mockindex import DEPENDENT, LIBRARY, MockIndex, run_cli_in, usage


def _get(url: str, headers: dict[str, str] = {}) -> tuple[int, bytes]:
    request = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(request) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def test_isolation_env() -> None:
    assert [v for v in os.environ if v.startswith("SYSAND_")] == [
        "SYSAND_TEST_CREDENTIAL_STORE"
    ]
    assert os.environ["SYSAND_TEST_CREDENTIAL_STORE"] == ":absent:"


def test_isolation_sweeps_ambient_vars(monkeypatch: pytest.MonkeyPatch) -> None:
    # Any `SYSAND_*` variable, including ones sysand does not know yet, is
    # removed rather than only a fixed list of today's names.
    monkeypatch.setenv("SYSAND_FUTURE_KNOB", "1")
    monkeypatch.setenv("SYSAND_CRED_AMBIENT", "https://example.invalid/*")
    with pytest.MonkeyPatch.context() as inner:
        isolate_sysand_env(inner)
        assert [v for v in os.environ if v.startswith("SYSAND_")] == [
            "SYSAND_TEST_CREDENTIAL_STORE"
        ]
    assert os.environ["SYSAND_FUTURE_KNOB"] == "1"


def test_layout_paths(mock_index: MockIndex) -> None:
    mock_index.publish(LIBRARY, "0.10.3", files={"library.sysml": b"package Library;"})
    mock_index.publish(LIBRARY, "0.11.0", usage=[usage(DEPENDENT, ">=1.0.0")])
    mock_index.publish("urn:kpar:other", "1.2.3")

    base = mock_index.url.rstrip("/")

    status, _ = _get(base + "/sysand-index-config.json")
    assert status == 404

    status, body = _get(base + "/mock/library/versions.json")
    assert status == 200
    versions = json.loads(body)["versions"]
    assert [v["version"] for v in versions] == ["0.11.0", "0.10.3"], (
        "ingest validation requires descending semver order"
    )
    assert versions[0]["usage"] == [
        {"resource": DEPENDENT, "versionConstraint": ">=1.0.0"}
    ]
    assert versions[1]["usage"] == []

    status, kpar = _get(base + "/mock/library/0.10.3/project.kpar")
    assert status == 200
    assert versions[1]["kpar_size"] == len(kpar)
    assert versions[1]["kpar_digest"] == "sha256:" + hashlib.sha256(kpar).hexdigest()
    assert versions[1]["kpar_digest"] == mock_index.kpar_digest(LIBRARY, "0.10.3")

    status, body = _get(base + "/mock/library/0.10.3/.project.json")
    assert json.loads(body) == {
        "name": "library",
        "version": "0.10.3",
        "publisher": "mock",
    }
    status, body = _get(base + "/mock/library/0.11.0/.project.json")
    assert json.loads(body)["usage"] == versions[0]["usage"]

    digest = hashlib.sha256(b"urn:kpar:other").hexdigest()
    status, body = _get(f"{base}/_iri/{digest}/versions.json")
    assert status == 200
    assert [v["version"] for v in json.loads(body)["versions"]] == ["1.2.3"]
    status, body = _get(f"{base}/_iri/{digest}/1.2.3/.project.json")
    assert json.loads(body) == {"name": "other", "version": "1.2.3"}

    status, _ = _get(base + "/mock/library/9.9.9/project.kpar")
    assert status == 404

    assert mock_index.requests("*/versions.json") == [
        "/mock/library/versions.json",
        f"/_iri/{digest}/versions.json",
    ]


def test_publish_rejects_duplicate_and_non_semver(mock_index: MockIndex) -> None:
    mock_index.publish(LIBRARY, "0.10.3")
    with pytest.raises(ValueError):
        mock_index.publish(LIBRARY, "0.10.3")
    with pytest.raises(ValueError):
        mock_index.publish(LIBRARY, "not-a-version")


def test_cli_lock_and_sync_against_mock_index(baseline: Baseline) -> None:
    lockfile = baseline.lockfile.read_text()
    assert 'name = "library"' in lockfile
    assert 'version = "0.10.3"' in lockfile
    assert (
        baseline.index.kpar_digest(LIBRARY, "0.10.3").removeprefix("sha256:")
        in lockfile
    ), "lockfile must retain the advertised digest"

    env_toml = (baseline.env_dir / "env.toml").read_text()
    assert LIBRARY in env_toml

    [installed] = baseline.installed()
    assert installed.endswith("_0.10.3")
    assert (baseline.env_dir / "lib" / installed / "library.sysml").read_bytes() == (
        b"package Library;\n"
    )

    # The whole 0.10.3 state came from the index, not from anywhere else.
    assert baseline.index.requests("*/project.kpar") == [
        "/mock/library/0.10.3/project.kpar"
    ]
    assert baseline.index.requests("/index.json") == []


def test_baseline_with_dependent(
    make_baseline: MakeBaseline, mock_index: MockIndex
) -> None:
    # A bare version constraint is caret in sysand: "0.10.1" is
    # `>=0.10.1, <0.11.0`, satisfied by 0.10.3.
    mock_index.publish(DEPENDENT, "1.0.0", usage=[usage(LIBRARY, "0.10.1")])
    baseline = make_baseline(extra_usages=[usage(DEPENDENT, "1.0.0")])
    installed = baseline.installed()
    assert len(installed) == 2
    assert any(n.endswith("_1.0.0") for n in installed)
    assert any(n.endswith("_0.10.3") for n in installed)


def test_require_bearer(mock_index: MockIndex, baseline: Baseline) -> None:
    mock_index.require_bearer("s3cret")
    versions_url = mock_index.url.rstrip("/") + MockIndex.versions_path(LIBRARY)

    status, _ = _get(versions_url)
    assert status == 401
    status, _ = _get(versions_url, {"Authorization": "Bearer wrong"})
    assert status == 401
    status, _ = _get(versions_url, {"Authorization": "Bearer s3cret"})
    assert status == 200

    # With the credential store simulated absent and no SYSAND_CRED_* set,
    # the CLI has nothing to authenticate with and must fail.
    (baseline.root / "sysand-lock.toml").unlink()
    assert baseline.cli("lock") is False
    assert not baseline.lockfile.exists()


def test_fail_next(mock_index: MockIndex) -> None:
    mock_index.publish(LIBRARY, "0.10.3")
    path = MockIndex.kpar_path(LIBRARY, "0.10.3")
    mock_index.fail_next(path, 500)
    url = mock_index.url.rstrip("/") + path
    assert _get(url)[0] == 500
    assert _get(url)[0] == 200
    assert mock_index.requests(path) == [path, path]


def test_run_cli_in_restores_cwd(tmp_path) -> None:
    before = os.getcwd()
    assert run_cli_in(tmp_path, "--version") is True
    assert os.getcwd() == before
