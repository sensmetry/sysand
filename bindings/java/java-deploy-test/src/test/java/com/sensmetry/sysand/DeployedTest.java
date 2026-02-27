// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand;

import static org.junit.jupiter.api.Assertions.*;

import java.util.regex.Pattern;
import org.junit.jupiter.api.Test;

public class DeployedTest {

    @Test
    public void testBasicInit() {
        try {
            java.nio.file.Path tempDir =
                java.nio.file.Files.createTempDirectory("sysand-test-init");
            // The original Sysand.init call is moved here and modified to use the
            // temporary directory.
            com.sensmetry.sysand.Sysand.init("test", "1.0.0", tempDir);

            // Add basic assertions to verify project creation
            assertTrue(
                java.nio.file.Files.exists(tempDir.resolve(".project.json")),
                "Project file should exist"
            );
            assertTrue(
                java.nio.file.Files.exists(tempDir.resolve(".meta.json")),
                "Metadata file should exist"
            );

            String projectJson = java.nio.file.Files.readString(
                tempDir.resolve(".project.json")
            );
            assertEquals(
                "{\n  \"name\": \"test\",\n  \"version\": \"1.0.0\",\n  \"usage\": []\n}",
                projectJson
            );

            String metaJson = java.nio.file.Files.readString(
                tempDir.resolve(".meta.json")
            );
            Pattern regex = Pattern.compile(
                "\\{\\s*\"index\":\\s*\\{\\},\\s*\"created\":\\s*\"\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}.\\d{6,9}Z\"\\s*\\}",
                Pattern.DOTALL
            );
            assertTrue(
                regex.matcher(metaJson).matches(),
                "Metadata file content should match expected pattern"
            );
        } catch (java.io.IOException e) {
            fail(
                "Failed during temporary directory operations or Sysand.init: " +
                    e.getMessage()
            );
        } catch (com.sensmetry.sysand.exceptions.SysandException e) {
            fail(
                "Failed during temporary directory operations or Sysand.init: " +
                    e.getMessage()
            );
        }
    }
}
