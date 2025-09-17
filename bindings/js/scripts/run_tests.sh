#!/bin/bash

set -e

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname $(realpath $0))
PACKAGE_DIR=$(dirname $SCRIPT_DIR)

cd $PACKAGE_DIR

WASM_BINDGEN_USE_BROWSER=1 wasm-pack test --headless --firefox
npm run test:browser