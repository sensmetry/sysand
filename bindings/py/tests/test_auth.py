# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`AuthPolicy`, `Resolution` and the typed exceptions, exercised through
`sysand.info()` against the mock index."""

from __future__ import annotations

import pytest

import sysand
from mockindex import MockIndex

IRI = "urn:kpar:auth_probe"


def resolution(index: MockIndex) -> sysand.Resolution:
    return sysand.Resolution(default_index=[index.url], use_config=False)


def test_auth_policy_repr_hides_secrets() -> None:
    policies = [
        sysand.AuthPolicy.none(),
        sysand.AuthPolicy.from_env(),
        sysand.AuthPolicy.from_env(keyring=False),
        sysand.AuthPolicy.bearer("https://example.org/**", "s3cret"),
        sysand.AuthPolicy.bearer("https://example.org/**", "s3cret", label="ci"),
        sysand.AuthPolicy.basic("https://example.org/**", "alice", "s3cret"),
    ]
    for policy in policies:
        assert "s3cret" not in repr(policy)
        assert "s3cret" not in str(policy)
    assert repr(policies[0]) == "AuthPolicy.none()"
    assert repr(policies[1]) == "AuthPolicy.from_env(keyring=True)"
    assert repr(policies[2]) == "AuthPolicy.from_env(keyring=False)"
    assert "https://example.org/**" in repr(policies[3])
    assert "label='ci'" in repr(policies[4])
    assert "alice" in repr(policies[5])
    assert sysand.AuthPolicy.bearer("g", "t").kind == "bearer"


def test_resolution_defaults_and_validation() -> None:
    default = sysand.Resolution()
    assert default.index == []
    assert default.default_index == []
    assert default.no_index is False
    assert default.include_std is False
    assert default.use_config is True
    assert "use_config=True" in repr(default)

    with pytest.raises(ValueError):
        sysand.Resolution(no_index=True, index=["https://example.org"])
    with pytest.raises(TypeError):
        sysand.Resolution(["https://example.org"])  # type: ignore[misc]


def test_exception_hierarchy() -> None:
    for cls in (
        sysand.ProjectError,
        sysand.ResolutionError,
        sysand.NotFoundError,
        sysand.SolveError,
        sysand.AuthError,
        sysand.IndexProtocolError,
        sysand.SyncError,
        sysand.EnvError,
    ):
        assert issubclass(cls, sysand.SysandError)
        assert issubclass(cls, RuntimeError)
        error = cls("message")
        assert error.wrote is False
        assert str(error) == "message"
    assert issubclass(sysand.NotFoundError, sysand.ResolutionError)

    solve = sysand.SolveError("no solution")
    assert solve.conflicts == []
    assert solve.report == "no solution"
    assert solve.kind == "no_solution"
    sync = sysand.SyncError("half", wrote=True, partial={"installed": [], "pruned": []})
    assert sync.wrote is True
    assert sync.partial == {"installed": [], "pruned": []}


def test_info_through_resolution(mock_index: MockIndex) -> None:
    mock_index.publish(IRI, "1.0.0")
    mock_index.publish(IRI, "1.1.0")

    info, _meta = sysand.info(IRI, resolution=resolution(mock_index))
    assert info["name"] == "auth_probe"
    assert info["version"] == "1.1.0"

    # The older spelling still works and means the same thing.
    info, _meta = sysand.info(IRI, index_urls=mock_index.url)
    assert info["version"] == "1.1.0"

    with pytest.raises(ValueError):
        sysand.info(IRI, index_urls=mock_index.url, resolution=resolution(mock_index))


def test_info_not_found(mock_index: MockIndex) -> None:
    with pytest.raises(sysand.NotFoundError) as excinfo:
        sysand.info("urn:kpar:absent", resolution=resolution(mock_index))
    assert isinstance(excinfo.value, sysand.ResolutionError)
    assert excinfo.value.wrote is False
    assert "urn:kpar:absent" in str(excinfo.value)


def test_info_auth_matrix(
    mock_index: MockIndex, monkeypatch: pytest.MonkeyPatch
) -> None:
    mock_index.publish(IRI, "1.0.0")
    mock_index.require_bearer("s3cret")
    res = resolution(mock_index)
    glob = mock_index.url + "**"

    # No credentials: the very first request (the discovery document) is
    # refused, and the error says so without naming any secret.
    with pytest.raises(sysand.AuthError) as excinfo:
        sysand.info(IRI, resolution=res)
    assert excinfo.value.wrote is False
    assert "401" in str(excinfo.value)
    assert mock_index.url in str(excinfo.value)
    assert "AuthPolicy" in str(excinfo.value)

    with pytest.raises(sysand.AuthError):
        sysand.info(IRI, resolution=res, auth=sysand.AuthPolicy.none())

    with pytest.raises(sysand.AuthError) as excinfo:
        sysand.info(IRI, resolution=res, auth=sysand.AuthPolicy.bearer(glob, "wrong"))
    assert glob in str(excinfo.value)
    assert "wrong" not in str(excinfo.value)

    with pytest.raises(sysand.AuthError):
        sysand.info(
            IRI, resolution=res, auth=sysand.AuthPolicy.basic(glob, "alice", "s3cret")
        )

    info, _meta = sysand.info(
        IRI, resolution=res, auth=sysand.AuthPolicy.bearer(glob, "s3cret")
    )
    assert info["version"] == "1.0.0"
    # An unauthenticated first attempt followed by the bearer retry.
    assert "/sysand-index-config.json" in mock_index.requests()

    # The CLI's own resolution from `SYSAND_CRED_*`, with and without the
    # (simulated absent) credential store.
    monkeypatch.setenv("SYSAND_CRED_TEST", glob)
    monkeypatch.setenv("SYSAND_CRED_TEST_BEARER_TOKEN", "s3cret")
    for policy in (
        sysand.AuthPolicy.from_env(keyring=False),
        sysand.AuthPolicy.from_env(),
    ):
        info, _meta = sysand.info(IRI, resolution=res, auth=policy)
        assert info["version"] == "1.0.0"

    # A wrong env token: the error names the variable, never the token.
    monkeypatch.setenv("SYSAND_CRED_TEST_BEARER_TOKEN", "wrong")
    with pytest.raises(sysand.AuthError) as excinfo:
        sysand.info(IRI, resolution=res, auth=sysand.AuthPolicy.from_env(keyring=False))
    assert "SYSAND_CRED_TEST" in str(excinfo.value)
    assert "wrong" not in str(excinfo.value)


def test_from_env_rejects_malformed_groups(
    mock_index: MockIndex, monkeypatch: pytest.MonkeyPatch
) -> None:
    mock_index.publish(IRI, "1.0.0")
    # A glob with no credential: the CLI refuses this too. `from_env()` is
    # pure configuration, so the error comes from the call that uses it.
    monkeypatch.setenv("SYSAND_CRED_TEST", mock_index.url + "**")
    policy = sysand.AuthPolicy.from_env(keyring=False)

    with pytest.raises(sysand.AuthError) as excinfo:
        sysand.info(IRI, resolution=resolution(mock_index), auth=policy)
    assert "SYSAND_CRED_TEST" in str(excinfo.value)
    assert excinfo.value.wrote is False
    assert mock_index.requests() == [], "a malformed credential set makes no request"
