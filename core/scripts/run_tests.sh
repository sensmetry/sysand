#!/bin/bash

# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

set -eu

# Compute the root directory based on the location of this script.
SCRIPT_DIR=$(dirname "$(realpath "$0")")
PACKAGE_DIR=$(dirname "$SCRIPT_DIR")

cd "$PACKAGE_DIR"

cargo test --features filesystem,networking,alltests $@
# Currently these features don't enable any tests
# cargo test --features js $@
# cargo test --features python $@
