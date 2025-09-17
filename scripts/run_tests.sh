#!/bin/bash

set -e

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname $(realpath $0))
ROOT_DIR=$(dirname $SCRIPT_DIR)

$ROOT_DIR/core/scripts/run_tests.sh

$ROOT_DIR/sysand/scripts/run_tests.sh

$ROOT_DIR/bindings/py/scripts/run_tests.sh

$ROOT_DIR/bindings/js/scripts/run_tests.sh

$ROOT_DIR/bindings/java/scripts/run_tests.sh