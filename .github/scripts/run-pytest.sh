#!/bin/sh

# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

# This script expects to be executed in CI from the root of the repository.
# Usage: .github/scripts/run-pytest.sh

ACTIVATION_SCRIPT=${ACTIVATION_SCRIPT:-.venv/bin/activate}

export UV_PYTHON_DOWNLOADS=automatic

set -ex

rm -f bindings/py/.python-version
uv python list --only-installed
cd bindings/py
uv venv --clear --no-project
. "${ACTIVATION_SCRIPT}"
uv sync --only-dev --active --no-install-project --locked --no-progress
uv pip install sysand --find-links ../../dist --force-reinstall --no-index
pytest
