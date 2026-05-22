// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

import groovy.json.JsonSlurper

// Without BUILD_TAG env var, only the untagged kpar should be produced
def taggedKpar = new File(basedir, "output/project1-1.0.0-dev.99.kpar")
assert !taggedKpar.exists() : "Unexpected tagged kpar found: ${taggedKpar}"

def kparFile = new File(basedir, "output/project1-1.0.0.kpar")
assert kparFile.exists() : "Expected untagged kpar not found: ${kparFile}"
