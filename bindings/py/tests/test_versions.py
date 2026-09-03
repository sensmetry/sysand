# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`sysand.versions()` against the mock index.

Every call here passes an explicit `resolution=`: the default `Resolution()`
means the CLI's configuration files plus the public default index, which no
test may touch."""

from __future__ import annotations

import json

import pytest

import sysand
from mockindex import LIBRARY, MockIndex

IRI = "urn:kpar:versions_probe"


def resolution(index: MockIndex) -> sysand.Resolution:
    return sysand.Resolution(default_index=[index.url], use_config=False)


def test_versions_lists_highest_first_with_one_request(mock_index: MockIndex) -> None:
    mock_index.publish(IRI, "0.10.3")
    mock_index.publish(IRI, "1.0.0")
    mock_index.publish(IRI, "0.11.0")

    listing = sysand.versions(IRI, resolution=resolution(mock_index))

    assert listing == {
        "iri": IRI,
        "versions": ["1.0.0", "0.11.0", "0.10.3"],
        "ignored": [],
    }
    assert mock_index.requests("*/versions.json") == [MockIndex.versions_path(IRI)]
    # The listing is answered from `versions.json` alone.
    assert mock_index.requests("*/.project.json") == []
    assert mock_index.requests("*/.meta.json") == []
    assert mock_index.requests("*/project.kpar") == []


def test_versions_of_a_purl(mock_index: MockIndex) -> None:
    mock_index.publish(LIBRARY, "0.10.3")

    listing = sysand.versions(LIBRARY, resolution=resolution(mock_index))

    assert listing["iri"] == LIBRARY
    assert listing["versions"] == ["0.10.3"]


def test_versions_not_found(mock_index: MockIndex) -> None:
    mock_index.publish(IRI, "1.0.0")

    with pytest.raises(sysand.NotFoundError) as excinfo:
        sysand.versions("urn:kpar:absent", resolution=resolution(mock_index))
    assert excinfo.value.wrote is False

    with pytest.raises(sysand.NotFoundError):
        sysand.versions(IRI, resolution=sysand.Resolution(no_index=True))
    assert mock_index.requests("*/versions.json") == [
        MockIndex.versions_path("urn:kpar:absent")
    ]


def test_versions_auth(mock_index: MockIndex, monkeypatch: pytest.MonkeyPatch) -> None:
    mock_index.publish(IRI, "1.0.0")
    mock_index.require_bearer("s3cret")
    res = resolution(mock_index)
    glob = mock_index.url + "**"

    with pytest.raises(sysand.AuthError):
        sysand.versions(IRI, resolution=res)

    listing = sysand.versions(
        IRI, resolution=res, auth=sysand.AuthPolicy.bearer(glob, "s3cret")
    )
    assert listing["versions"] == ["1.0.0"]

    monkeypatch.setenv("SYSAND_CRED_TEST", glob)
    monkeypatch.setenv("SYSAND_CRED_TEST_BEARER_TOKEN", "s3cret")
    listing = sysand.versions(
        IRI, resolution=res, auth=sysand.AuthPolicy.from_env(keyring=False)
    )
    assert listing["versions"] == ["1.0.0"]


def test_versions_index_protocol_error(mock_index: MockIndex) -> None:
    mock_index.publish(IRI, "1.0.0")
    path = MockIndex.versions_path(IRI)
    good = json.loads(
        json.dumps(
            {
                "versions": [
                    {
                        "version": v,
                        "usage": [],
                        "kpar_size": mock_index.kpar_size(IRI, "1.0.0"),
                        "kpar_digest": mock_index.kpar_digest(IRI, "1.0.0"),
                    }
                    for v in ("1.0.0", "2.0.0")
                ]
            }
        )
    )
    # Valid JSON that violates the protocol: ascending order.
    mock_index.override(path, json.dumps(good).encode())
    with pytest.raises(sysand.IndexProtocolError) as excinfo:
        sysand.versions(IRI, resolution=resolution(mock_index))
    assert excinfo.value.wrote is False

    mock_index.override(path, b"{not json")
    with pytest.raises(sysand.IndexProtocolError):
        sysand.versions(IRI, resolution=resolution(mock_index))
