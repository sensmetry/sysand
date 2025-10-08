#!/bin/bash

set -e

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname $(realpath $0))
PACKAGE_DIR=$(dirname $SCRIPT_DIR)

cd $PACKAGE_DIR

export LD_LIBRARY_PATH=$(python -c "from sysconfig import get_config_var as s; print(s(\"LIBDIR\"))")

# Workaround to make pyo3 detect venv python libs on macOS.
# See https://github.com/PyO3/pyo3/issues/1741
export PYTHONHOME="$(python -c 'import sys; print(sys.base_prefix)')"

maturin develop

cargo test --no-default-features

uv run pytest
