#!/bin/bash

set -e

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname $(realpath $0))
PACKAGE_DIR=$(dirname $SCRIPT_DIR)

cd $PACKAGE_DIR

cargo fmt
cargo clippy --all-targets -- --deny warnings
uv sync --group linters
uv run ruff format python tests
uv run ruff check python
uv run mypy --strict python
