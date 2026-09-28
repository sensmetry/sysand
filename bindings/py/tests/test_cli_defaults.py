# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""Defaults that match the CLI's.

`info` and `versions` resolve as `sysand info --iri` does, from the current
directory: the enclosing project's environment, the configuration's
overrides, then the indexes. `sources` lists dependency sources by default,
and `build` refuses `file://` usages unless told otherwise.

The user configuration file is redirected to an empty directory, and every
index is the mock one, so no test reaches the public index."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import sysand
from mockindex import MockIndex, usage

IRI = "urn:kpar:defaults_probe"
NO_INDEX = sysand.Resolution(no_index=True)


@pytest.fixture(autouse=True)
def _no_user_config(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "user-config"))


def project(root: Path, name: str = "proj") -> Path:
    root.mkdir(parents=True)
    sysand.init(project_dir=root, name=name, publisher="acme", version="1.0.0")
    return root


def test_info_and_versions_default_to_the_configured_indexes(
    tmp_path: Path, mock_index: MockIndex, monkeypatch: pytest.MonkeyPatch
) -> None:
    mock_index.publish(IRI, "1.0.0")
    mock_index.publish(IRI, "1.1.0")
    root = project(tmp_path / "proj")
    (root / "sysand.toml").write_text(
        f'[[index]]\nurl = "{mock_index.url}"\ndefault = true\n'
    )
    # As in the CLI, the configuration is the enclosing project's, found
    # from anywhere inside it.
    inside = root / "sub"
    inside.mkdir()
    monkeypatch.chdir(inside)

    info, _meta = sysand.info(iri=IRI)
    assert info["version"] == "1.1.0"
    assert sysand.versions(iri=IRI)["versions"] == ["1.1.0", "1.0.0"]

    # `no_index` still turns the index off.
    with pytest.raises(sysand.NotFoundError):
        sysand.info(iri=IRI, resolution=NO_INDEX)


def test_info_and_versions_apply_configured_overrides(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = project(tmp_path / "proj")
    project(root / "local" / "over", name="over")
    (root / "sysand.toml").write_text(
        '[[project]]\nidentifiers = ["urn:kpar:over"]\n'
        'sources = [{ src_path = "local/over" }]\n'
    )
    monkeypatch.chdir(root)

    info, _meta = sysand.info(iri="urn:kpar:over", resolution=NO_INDEX)
    assert info["name"] == "over"
    assert sysand.versions(iri="urn:kpar:over", resolution=NO_INDEX)["versions"] == [
        "1.0.0"
    ]

    # Overrides are configuration: without it they do not apply.
    without_config = sysand.Resolution(no_index=True, use_config=False)
    with pytest.raises(sysand.NotFoundError):
        sysand.info(iri="urn:kpar:over", resolution=without_config)


def test_info_reads_the_project_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = project(tmp_path / "proj")
    installed = project(tmp_path / "installed", name="installed")
    env_dir = root / sysand.env.DEFAULT_ENV_NAME
    sysand.env.env(path=env_dir)
    sysand.env.install_path(
        env_path=env_dir, iri="urn:kpar:installed", location=installed
    )

    monkeypatch.chdir(root)
    info, _meta = sysand.info(iri="urn:kpar:installed", resolution=NO_INDEX)
    assert info["name"] == "installed"

    # Outside the project its environment is not consulted.
    monkeypatch.chdir(tmp_path)
    with pytest.raises(sysand.NotFoundError):
        sysand.info(iri="urn:kpar:installed", resolution=NO_INDEX)


def test_sources_finds_the_project_environment(tmp_path: Path) -> None:
    root = project(tmp_path / "proj")
    (root / "own.sysml").write_text("package Own;")
    sysand.include(project_dir=root, src_path="own.sysml")

    dep = project(tmp_path / "dep", name="dep")
    (dep / "dep.sysml").write_text("package Dep;")
    sysand.include(project_dir=dep, src_path="dep.sysml")

    env_dir = root / sysand.env.DEFAULT_ENV_NAME
    sysand.env.env(path=env_dir)
    sysand.env.install_path(env_path=env_dir, iri="urn:kpar:dep", location=dep)
    sysand.add(project_dir=root, iri="urn:kpar:dep", version_constraint="1.0.0")

    names = sorted(Path(p).name for p in sysand.sources(project_dir=root))
    assert names == ["dep.sysml", "own.sysml"]

    [dep_source] = sysand.env.sources(env_path=env_dir, iri="urn:kpar:dep")
    assert Path(dep_source).name == "dep.sysml"


def test_sources_without_environment_needs_one_for_dependencies(
    tmp_path: Path,
) -> None:
    root = project(tmp_path / "proj")
    manifest = json.loads((root / ".project.json").read_text())
    manifest["usage"] = [usage("urn:kpar:missing", "1.0.0")]
    (root / ".project.json").write_text(json.dumps(manifest))

    with pytest.raises(RuntimeError):
        sysand.sources(project_dir=root)
    assert sysand.sources(project_dir=root, dependencies=sysand.Dependencies.NONE) == []


def test_build_refuses_path_usages_unless_allowed(tmp_path: Path) -> None:
    root = project(tmp_path / "proj")
    elsewhere = project(tmp_path / "elsewhere", name="elsewhere")
    manifest = json.loads((root / ".project.json").read_text())
    manifest["usage"] = [usage(elsewhere.as_uri())]
    (root / ".project.json").write_text(json.dumps(manifest))

    with pytest.raises(ValueError, match="allow_path_usage=True"):
        sysand.build(output_path=tmp_path / "refused.kpar", project_dir=root)
    assert not (tmp_path / "refused.kpar").exists()

    sysand.build(
        output_path=tmp_path / "allowed.kpar", project_dir=root, allow_path_usage=True
    )
    assert (tmp_path / "allowed.kpar").is_file()
