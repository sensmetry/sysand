// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

import java.util.zip.ZipFile
import groovy.json.JsonSlurper

def kparFile = new File(basedir, "output/project1-1.0.0.kpar")
assert kparFile.exists() : "Expected kpar file not found: ${kparFile}"

def zip = new ZipFile(kparFile)
try {
    def infoEntry = zip.getEntry(".project.json")
    assert infoEntry != null : ".project.json entry not found in kpar"

    def infoJson = new JsonSlurper().parse(zip.getInputStream(infoEntry))
    assert infoJson.version == "1.0.0" :
        "Expected version '1.0.0' but got '${infoJson.version}'"

    def metaEntry = zip.getEntry(".meta.json")
    assert metaEntry != null : ".meta.json entry not found in kpar"

    def metaJson = new JsonSlurper().parse(zip.getInputStream(metaEntry))
    assert metaJson.metamodel == "https://www.omg.org/spec/KerML/20250201" :
        "Expected metamodel 'https://www.omg.org/spec/KerML/20250201' but got '${metaJson.metamodel}'"
} finally {
    zip.close()
}
