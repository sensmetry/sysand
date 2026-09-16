# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""`sysand.lock` inside a workspace.

Locking a member locks the whole workspace at its root, as `sysand
lock` does: a member has no lockfile of its own.
"""

from __future__ import annotations

from pathlib import Path

import pytest

import sysand

# `sysand.init` takes a metamodel: a project declares one and it
# cannot be guessed.
SYSML = "https://www.omg.org/spec/SysML/20250201"

OFFLINE = sysand.Resolution(no_index=True)


def workspace(root: Path, members: dict[str, str]) -> None:
    """A `model.toml` workspace. `members` maps a directory to the extra
    manifest body its project carries."""
    listed = ", ".join(f'"{name}"' for name in members)
    (root / "model.toml").write_text(
        f'manifest-version = "1.0.0"\n\n[workspace]\nmembers = [{listed}]\n'
    )
    for name, extra in members.items():
        member = root / name
        member.mkdir(parents=True, exist_ok=True)
        (member / "model.toml").write_text(
            'manifest-version = "1.0.0"\n\n[project]\n'
            f'publisher = "acme"\nname = "{name}"\nversion = "1.0.0"\n'
            f'sources = []\nmetamodel = "{SYSML}"\n{extra}'
        )


def uses(name: str) -> str:
    return (
        f'\n[[inline-usages]]\npublisher = "acme"\nname = "{name}"\ndir = "../{name}"\n'
    )


def test_a_member_lock_writes_at_the_workspace_root(tmp_path: Path) -> None:
    workspace(tmp_path, {"app": uses("dep"), "dep": ""})

    sysand.lock(tmp_path / "app", resolution=OFFLINE)

    assert (tmp_path / "sysand-lock.toml").is_file()
    assert not (tmp_path / "app" / "sysand-lock.toml").exists()


def test_a_member_lock_solves_every_member(tmp_path: Path) -> None:
    """The location is not enough: locking a member alone into the right
    file would still be the wrong solve.

    `other` is the discriminator -- it is a member but *not* a
    dependency of `app`, so a member-only solve cannot reach it. A
    fixture where every member is a dependency proves nothing.
    """
    workspace(tmp_path, {"app": uses("dep"), "dep": "", "other": ""})

    result = sysand.lock(tmp_path / "app", resolution=OFFLINE)

    assert {p["name"] for p in result["projects"]} == {"app", "dep", "other"}


def test_a_workspace_root_can_be_locked(tmp_path: Path) -> None:
    """A workspace root is not a project, but it can be locked."""
    workspace(tmp_path, {"app": uses("dep"), "dep": ""})

    result = sysand.lock(tmp_path, resolution=OFFLINE)

    assert (tmp_path / "sysand-lock.toml").is_file()
    assert {p["name"] for p in result["projects"]} == {"app", "dep"}


def test_a_standalone_project_locks_at_its_own_root(tmp_path: Path) -> None:
    """The workspace is used when there is one, not always."""
    sysand.init("standalone", "acme", "1.0.0", tmp_path, metamodel=SYSML)

    sysand.lock(tmp_path, resolution=OFFLINE)

    assert (tmp_path / "sysand-lock.toml").is_file()


def test_an_unlisted_project_locks_at_its_own_root(tmp_path: Path) -> None:
    """A project under a workspace root but absent from `members` is not
    in the workspace -- and that is an answer, not an error: the project
    is locked on its own."""
    workspace(tmp_path, {"app": ""})
    unlisted = tmp_path / "vendored"
    unlisted.mkdir()
    sysand.init("vendored", "acme", "1.0.0", unlisted, metamodel=SYSML)

    sysand.lock(unlisted, resolution=OFFLINE)

    assert (unlisted / "sysand-lock.toml").is_file()
    assert not (tmp_path / "sysand-lock.toml").exists()


def test_an_unlisted_project_reads_the_workspace_configuration(tmp_path: Path) -> None:
    """Configuration is read at the *config* root, which is the
    discovered workspace root even for a project that is not a member.

    The member case cannot show this: a member's lockfile is at the
    workspace root, so the two roots coincide there. An unlisted project
    is where they part -- its lockfile is its own, its configuration is
    the workspace's -- since the config root comes from discovery and
    not from membership.
    """
    workspace(tmp_path, {"app": ""})
    unlisted = tmp_path / "vendored"
    unlisted.mkdir()
    sysand.init("vendored", "acme", "1.0.0", unlisted, metamodel=SYSML)
    # An index URL the config parser refuses, so reading it is visible.
    (tmp_path / "sysand.toml").write_text(
        '[[index]]\nurl = "https://example.com/#frag"\n'
    )

    with pytest.raises(sysand.ProjectError):
        sysand.lock(unlisted, resolution=sysand.Resolution())
