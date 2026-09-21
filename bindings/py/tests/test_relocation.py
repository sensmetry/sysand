# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""The shipped Python tree must work when vendored under another package.

Consumers copy `python/sysand/` in as a subpackage of their own and supply a
`_sysand_core` shim pointing at wherever they compiled the extension. This
rehearses that: the tree is copied to `vendor/project/`, given a shim, and
imported in a subprocess where the name `sysand` is not importable at all.

A subprocess rather than a fixture, because `conftest.py` imports `sysand` at
session scope; a cached `sys.modules` entry would bypass any in-process block
and the test would silently prove nothing.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

import sysand._sysand_core as _sysand_core  # type: ignore

PACKAGE = Path(__file__).parents[1] / "python" / "sysand"

# Loaded with an explicit `ExtensionFileLoader`: the compiled file is
# `_sysand_core.abi3.so` (`.pyd` on Windows), and suffix inference over
# `.abi3.so` is not something to rely on. Replacing `sys.modules[__name__]`
# rather than re-exporting, so that the underscore names the tree needs
# (`_run_cli`, `_register_errors`) come across too.
_SHIM = """\
import importlib.util
import os
import sys
from importlib.machinery import ExtensionFileLoader

_path = os.environ["SYSAND_EXTENSION_PATH"]
_loader = ExtensionFileLoader(__name__, _path)
_spec = importlib.util.spec_from_loader(__name__, _loader, origin=_path)
_module = importlib.util.module_from_spec(_spec)
_loader.exec_module(_module)
sys.modules[__name__] = _module
"""

_DRIVER = '''\
import pathlib
import sys


class _Blocker:
    """Make the name this tree used to import by absolute path unavailable."""

    def find_spec(self, fullname, path=None, target=None):
        if fullname == "sysand" or fullname.startswith("sysand."):
            raise ImportError(f"blocked: {fullname}")
        return None


sys.meta_path.insert(0, _Blocker())

import vendor.project as project

# A call that reaches the extension and succeeds.
project.init(
    name="relocated", publisher="a", version="1.0.0", project_dir=sys.argv[1]
)
assert (
    pathlib.Path(sys.argv[1]) / ".project.json"
).is_file(), "init did not write the project"

# A call that fails, to pin the error classes to *this* copy of the tree.
try:
    project.set_usage_constraint(
        project_dir=sys.argv[2], iri="pkg:sysand/example", version_constraint="1.0.0"
    )
except project.ProjectError:
    pass
else:
    raise AssertionError("expected a ProjectError from the relocated tree")
'''


def _vendor(tmp_path: Path) -> Path:
    root = tmp_path / "tree"
    package = root / "vendor" / "project"
    package.parent.mkdir(parents=True)
    (root / "vendor" / "__init__.py").write_text("")
    shutil.copytree(
        PACKAGE,
        package,
        ignore=shutil.ignore_patterns("__pycache__", "_sysand_core.*"),
    )
    (package / "_sysand_core.py").write_text(_SHIM)
    (root / "driver.py").write_text(_DRIVER)
    return root


def test_tree_works_vendored_under_another_package(tmp_path: Path) -> None:
    root = _vendor(tmp_path)
    project_dir = tmp_path / "project"
    project_dir.mkdir()
    empty_dir = tmp_path / "empty"
    empty_dir.mkdir()

    # Resolved here, not in the driver: the driver has just made `sysand`
    # unimportable, so it cannot look the extension up for itself.
    extension = _sysand_core.__file__
    assert extension is not None

    result = subprocess.run(
        [sys.executable, str(root / "driver.py"), str(project_dir), str(empty_dir)],
        cwd=root,
        env={
            **os.environ,
            "SYSAND_EXTENSION_PATH": extension,
            "PYTHONPATH": str(root),
        },
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, (
        "the vendored tree did not work standalone:\n"
        f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
    )
