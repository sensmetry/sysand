// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

import java.util.zip.ZipFile
import groovy.json.JsonSlurper

// Scenario 1: BUILD_TAG=99 was set — tagged kpar must exist with the dev suffix.
def taggedKpar = new File(basedir, "output/project1-1.0.0-dev.99.kpar")
assert taggedKpar.exists() : "Expected tagged kpar not found: ${taggedKpar}"

def zip = new ZipFile(taggedKpar)
try {
    def infoEntry = zip.getEntry(".project.json")
    assert infoEntry != null : ".project.json entry not found in tagged kpar"

    def infoJson = new JsonSlurper().parse(zip.getInputStream(infoEntry))
    assert infoJson.version == "1.0.0-dev.99" :
        "Expected version '1.0.0-dev.99' in tagged kpar but got '${infoJson.version}'"
} finally {
    zip.close()
}

// Scenario 2: BUILD_TAG was absent — untagged kpar must exist with the base version.
def untaggedKpar = new File(basedir, "output/project1-1.0.0.kpar")
assert untaggedKpar.exists() : "Expected untagged kpar not found: ${untaggedKpar}"

def zip2 = new ZipFile(untaggedKpar)
try {
    def infoEntry = zip2.getEntry(".project.json")
    assert infoEntry != null : ".project.json entry not found in untagged kpar"

    def infoJson = new JsonSlurper().parse(zip2.getInputStream(infoEntry))
    assert infoJson.version == "1.0.0" :
        "Expected version '1.0.0' in untagged kpar but got '${infoJson.version}'"
} finally {
    zip2.close()
}

// Source .project.json must not have been modified by either build.
def sourceInfo = new JsonSlurper().parse(new File(basedir, "project1/.project.json"))
assert sourceInfo.version == "1.0.0" :
    "Source .project.json was unexpectedly modified: version is '${sourceInfo.version}'"
