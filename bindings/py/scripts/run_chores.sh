#!/bin/bash

set -e

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname $(realpath $0))
PACKAGE_DIR=$(dirname $SCRIPT_DIR)

cd $PACKAGE_DIR

cargo fmt
cargo clippy --all-targets -- --deny warnings
uv tool run ruff format python tests
uv tool run ruff check python
uv tool run mypy --strict python