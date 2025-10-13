#!/bin/bash

set -eu

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname "$(realpath "$0")")
PACKAGE_DIR=$(dirname "$SCRIPT_DIR")

cd "$PACKAGE_DIR"

cargo test --features filesystem,networking,alltests
cargo test --features js
source ../scripts/py_path.sh
cargo test --features python
