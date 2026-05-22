// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

import java.util.zip.ZipFile
import groovy.json.JsonSlurper

def kparFile = new File(basedir, "output/project1-1.0.0-dev.42.kpar")
assert kparFile.exists() : "Expected tagged kpar not found: ${kparFile}"

def zip = new ZipFile(kparFile)
try {
    def infoEntry = zip.getEntry(".project.json")
    assert infoEntry != null : ".project.json entry not found in kpar"

    def infoJson = new JsonSlurper().parse(zip.getInputStream(infoEntry))
    assert infoJson.version == "1.0.0-dev.42" :
        "Expected version '1.0.0-dev.42' in kpar but got '${infoJson.version}'"
} finally {
    zip.close()
}

// Source .project.json must not have been modified by the build
def sourceInfo = new JsonSlurper().parse(new File(basedir, "project1/.project.json"))
assert sourceInfo.version == "1.0.0" :
    "Source .project.json was unexpectedly modified: version is '${sourceInfo.version}'"
