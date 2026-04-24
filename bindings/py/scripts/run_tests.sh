#!/bin/bash

# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

set -eu

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname "$(realpath "$0")")
PACKAGE_DIR=$(dirname "$SCRIPT_DIR")

cd "$PACKAGE_DIR"

# Isolate from an outer conda environment. maturin refuses to run when both
# CONDA_PREFIX and VIRTUAL_ENV are set, and `uv run` manages the project's
# .venv itself — no conda involvement is needed or wanted.
unset CONDA_PREFIX CONDA_DEFAULT_ENV CONDA_PROMPT_MODIFIER CONDA_SHLVL

uv run maturin develop

source ../../scripts/py_path.sh
cargo test --no-default-features

uv run pytest
