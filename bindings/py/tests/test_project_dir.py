# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""The functions that act on a project take the project's own directory,
and act as the CLI does in a workspace."""

from __future__ import annotations

from pathlib import Path

import pytest

import sysand

SYSML = "https://www.omg.org/spec/SysML/20250201"

WORKSPACE = 'manifest-version = "1.0"\n\n[workspace]\nmembers = ["app"]\n'


def workspace(root: Path) -> Path:
    """A workspace with one member `app`."""
    (root / "model.toml").write_text(WORKSPACE)
    app = root / "app"
    (app / "src").mkdir(parents=True)
    (app / "model.toml").write_text(
        'manifest-version = "1.0"\n\n[project]\n'
        'publisher = "acme"\nname = "app"\nversion = "1.0.0"\n'
        f'sources = []\nmetamodel = "{SYSML}"\n'
    )
    return app


def init_member(root: Path, at: str, name: str) -> None:
    sysand.init(
        project_dir=root / at,
        name=name,
        publisher="acme",
        version="1.0.0",
        metamodel=SYSML,
        kind=sysand.ProjectKind.MODEL_TOML,
    )


def test_init_in_a_workspace_appends_the_member(tmp_path: Path) -> None:
    workspace(tmp_path)

    init_member(tmp_path, "lib", "lib")

    assert '"lib"' in (tmp_path / "model.toml").read_text()
    assert (tmp_path / "lib" / "model.toml").exists()


def test_init_refuses_a_member_the_workspace_would_not_load_with(
    tmp_path: Path,
) -> None:
    workspace(tmp_path)

    # The same project as `app`.
    with pytest.raises(sysand.ProjectError, match="name the same project"):
        init_member(tmp_path, "lib", "app")

    assert '"lib"' not in (tmp_path / "model.toml").read_text()
    assert not (tmp_path / "lib").exists()


def test_init_refuses_a_json_project_the_workspace_would_list(
    tmp_path: Path,
) -> None:
    workspace(tmp_path)

    with pytest.raises(sysand.ProjectError, match="must be a `model.toml` project"):
        sysand.init(
            project_dir=tmp_path / "lib",
            name="lib",
            publisher="acme",
            version="1.0.0",
            metamodel=SYSML,
            kind=sysand.ProjectKind.PROJECT_JSON,
        )

    assert '"lib"' not in (tmp_path / "model.toml").read_text()
    assert not (tmp_path / "lib").exists()


def test_a_subdirectory_of_a_project_is_not_the_project(tmp_path: Path) -> None:
    app = workspace(tmp_path)

    with pytest.raises(sysand.ProjectError, match="not the root of a project"):
        sysand.include(project_dir=app / "src", src_path="a.sysml")
    with pytest.raises(sysand.ProjectError, match="not the root of a project"):
        sysand.info_path(project_dir=app / "src")
    with pytest.raises(sysand.ProjectError, match="not the root of a project"):
        sysand.sources(project_dir=app / "src")


def test_the_workspace_root_is_not_a_project(tmp_path: Path) -> None:
    workspace(tmp_path)

    with pytest.raises(sysand.ProjectError, match="not the root of a project"):
        sysand.build(output_path=tmp_path / "out.kpar", project_dir=tmp_path)
    with pytest.raises(sysand.ProjectError, match="not the root of a project"):
        sysand.sources(project_dir=tmp_path)
    with pytest.raises(sysand.ProjectError, match="not the root of a project"):
        sysand.sources(project_dir=tmp_path, include_own=False)
