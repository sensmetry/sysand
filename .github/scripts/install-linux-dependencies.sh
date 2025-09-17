#!/bin/bash

# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

# This script is taken from
# https://emresahin.net/perl-error-building-rust-openssl-with-maturin-ubuntu-and-manylinux/

# If we're running on rhel centos, install needed packages.
if command -v yum &> /dev/null; then
    yum update -y && yum install -y perl-core openssl openssl-devel pkgconfig libatomic

    # If we're running on i686 we need to symlink libatomic
    # in order to build openssl with -latomic flag.
    if [[ ! -d "/usr/lib64" ]]; then
        ln -s /usr/lib/libatomic.so.1 /usr/lib/libatomic.so
    fi
else
    # If we're running on debian-based system.
    apt-get update -y && apt-get install -y libssl-dev openssl pkg-config
fi
