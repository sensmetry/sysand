#!/bin/bash

set -e

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname $(realpath $0))
PACKAGE_DIR=$(dirname $SCRIPT_DIR)

cd $PACKAGE_DIR

uv sync --only-dev

# Workaround to make pyo3 detect venv python libs on macOS.
# See https://github.com/PyO3/pyo3/issues/1741
export PYTHONHOME="$(uv run python -c 'import sys; print(sys.base_prefix)')"

uv run maturin develop

cargo test --no-default-features

uv run pytest
