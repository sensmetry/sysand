#!/bin/bash

# Assumes that Java, Maven, and Python are installed.

set -eu

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname "$(realpath "$0")")
PACKAGE_DIR=$(dirname "$SCRIPT_DIR")

cd "$PACKAGE_DIR"

cargo test --bins --tests
python3 scripts/java-builder.py build
python3 scripts/java-builder.py test
