#!/bin/bash

set -e

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname $(realpath $0))
PACKAGE_DIR=$(dirname $SCRIPT_DIR)

cd $PACKAGE_DIR

export LD_LIBRARY_PATH=$(python -c "from distutils.sysconfig import get_config_var as s; print(s(\"LIBDIR\"))")

maturin develop

cargo test --no-default-features

pytest