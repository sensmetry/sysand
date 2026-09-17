// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

import java.nio.file.Files;

/**
 * A smoke test against the *deployed* artifact, run by
 * {@code java-builder.py test-deployed} rather than by the library
 * suite. It exists to prove the published jar loads its native library
 * and that one end-to-end call works -- not to re-test behaviour the
 * library suite covers. This file compiles only during a release.
 */
public class DeployedTest {

    private static final String SYSML = "https://www.omg.org/spec/SysML/20250201";

    @Test
    public void testBasicInit() {
        try {
            java.nio.file.Path tempDir = Files.createTempDirectory("sysand-test-init");
            com.sensmetry.sysand.Sysand.init("test", "acme", "1.0.0", null, SYSML, tempDir);

            // `init` writes a `model.toml`; the two json documents are
            // what a build produces and what a legacy project has.
            assertTrue(Files.exists(tempDir.resolve("model.toml")), "Manifest should exist");
            assertFalse(Files.exists(tempDir.resolve(".project.json")), "No json manifest");
            assertFalse(Files.exists(tempDir.resolve(".meta.json")), "No json metadata");

            String manifest = new String(Files.readAllBytes(tempDir.resolve("model.toml")));
            assertEquals(
                    "manifest-version = \"1.0.0\"\n"
                            + "\n"
                            + "[project]\n"
                            + "publisher = \"acme\"\n"
                            + "name = \"test\"\n"
                            + "version = \"1.0.0\"\n"
                            + "sources = []\n"
                            + "metamodel = \"" + SYSML + "\"\n",
                    manifest);
        } catch (java.io.IOException e) {
            fail("Failed during temporary directory operations or Sysand.init: " + e.getMessage());
        } catch (com.sensmetry.sysand.exceptions.SysandException e) {
            fail("Failed during temporary directory operations or Sysand.init: " + e.getMessage());
        }
    }

}
