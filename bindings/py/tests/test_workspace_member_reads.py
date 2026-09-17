# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""Reading a member that *inherits* from its workspace.

A member's manifest resolves only against its workspace's table, so the
binding entry points that take a path must find that table for a member
they are given.
"""

from __future__ import annotations

from pathlib import Path

import pytest

import sysand

SYSML = "https://www.omg.org/spec/SysML/20250201"


def inheriting_workspace(root: Path) -> Path:
    """A workspace whose one member inherits its `metamodel`."""
    (root / "model.toml").write_text(
        'manifest-version = "1.0.0"\n\n[workspace]\nmembers = ["app"]\n\n'
        f'[workspace.group.sysml]\nmetamodel = "{SYSML}"\n'
    )
    member = root / "app"
    member.mkdir(parents=True, exist_ok=True)
    (member / "model.toml").write_text(
        'manifest-version = "1.0.0"\n\n[project]\n'
        'publisher = "acme"\nname = "member"\nversion = "1.0.0"\n'
        'sources = []\nmetamodel.workspace = "sysml"\n'
    )
    return member


def test_info_path_reads_an_inheriting_member(tmp_path: Path) -> None:
    member = inheriting_workspace(tmp_path)
    info, _meta = sysand.info_path(project_dir=str(member))
    assert info["name"] == "member"


def test_sources_reads_an_inheriting_member(tmp_path: Path) -> None:
    member = inheriting_workspace(tmp_path)
    # No sources to list; what matters is that the manifest resolves at
    # all.
    sysand.sources(project_dir=str(member))


def test_build_reads_an_inheriting_member(tmp_path: Path) -> None:
    member = inheriting_workspace(tmp_path)
    # `build` takes the output path first.
    sysand.build(output_path=str(tmp_path / "out.kpar"), project_dir=str(member))
    assert (tmp_path / "out.kpar").is_file()


def test_a_nested_project_is_not_given_the_table(tmp_path: Path) -> None:
    """`app/sub` sits inside a member and is not one.

    The table is granted on an exact `members` entry, not on a prefix:
    a project that merely sits inside a member is outside every
    workspace, and must fail exactly as it does from its own directory.
    """
    member = inheriting_workspace(tmp_path)
    nested = member / "sub"
    nested.mkdir()
    (nested / "model.toml").write_text(
        'manifest-version = "1.0.0"\n\n[project]\n'
        'publisher = "acme"\nname = "nested"\nversion = "1.0.0"\n'
        'sources = []\nmetamodel.workspace = "sysml"\n'
    )
    # Under the root, in neither list.
    with pytest.raises(
        Exception,
        match="neither listed in `workspace.members` nor under `workspace.exclude`",
    ):
        sysand.info_path(project_dir=str(nested))


def test_lock_refuses_an_unlisted_project(tmp_path: Path) -> None:
    """An unlisted project under a workspace root is an error, as in the CLI.

    It is not locked on its own: that would write a second lockfile and
    `.sysand` inside the workspace.
    """
    inheriting_workspace(tmp_path)
    unlisted = tmp_path / "unlisted"
    unlisted.mkdir()
    (unlisted / "model.toml").write_text(
        'manifest-version = "1.0.0"\n\n[project]\n'
        'publisher = "acme"\nname = "unlisted"\nversion = "1.0.0"\n'
        f'sources = []\nmetamodel = "{SYSML}"\n'
    )
    with pytest.raises(
        Exception,
        match="neither listed in `workspace.members` nor under `workspace.exclude`",
    ):
        sysand.lock(path=unlisted, write=False)
    assert not (unlisted / "sysand-lock.toml").exists()
