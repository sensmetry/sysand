#!/bin/bash

set -eu

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname "$(realpath "$0")")
PACKAGE_DIR=$(dirname "$SCRIPT_DIR")

cd "$PACKAGE_DIR"

uv run maturin develop

source ../../scripts/py_path.sh
cargo test --no-default-features

uv run pytest
