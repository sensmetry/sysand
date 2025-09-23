// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

import java.util.regex.Pattern;
import java.nio.file.Files;

public class BasicTest {

    @Test
    public void testBasicInit() {
        try {
            java.nio.file.Path tempDir = java.nio.file.Files.createTempDirectory("sysand-test-init");
            // The original Sysand.init call is moved here and modified to use the
            // temporary directory.
            com.sensmetry.sysand.Sysand.init("test", "1.0.0", tempDir);

            // Add basic assertions to verify project creation
            assertTrue(Files.exists(tempDir.resolve(".project.json")), "Project file should exist");
            assertTrue(Files.exists(tempDir.resolve(".meta.json")), "Metadata file should exist");

            // java.nio.file.Files.readString is available in Java 11+
            // String projectJson = java.nio.file.Files.readString(tempDir.resolve(".project.json"));
            String projectJson = new String(Files.readAllBytes(tempDir.resolve(".project.json")));
            assertEquals("{\n  \"name\": \"test\",\n  \"version\": \"1.0.0\",\n  \"usage\": []\n}", projectJson);

            // String metaJson = Files.readString(tempDir.resolve(".meta.json"));
            String metaJson = new String(Files.readAllBytes(tempDir.resolve(".meta.json")));
            Pattern regex = Pattern.compile(
                    "\\{\\s*\"index\":\\s*\\{\\},\\s*\"created\":\\s*\"\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}.\\d{6,9}Z\"\\s*\\}",
                    Pattern.DOTALL);
            assertTrue(regex.matcher(metaJson).matches(), "Metadata file content should match expected pattern");
        } catch (java.io.IOException e) {
            fail("Failed during temporary directory operations or Sysand.init: " + e.getMessage());
        } catch (com.sensmetry.sysand.exceptions.SysandException e) {
            fail("Failed during temporary directory operations or Sysand.init: " + e.getMessage());
        }
    }

    @Test
    public void testBasicEnv() {
        try {
            java.nio.file.Path tempDir = java.nio.file.Files.createTempDirectory("sysand-test-env");
            java.nio.file.Path envPath = tempDir.resolve(com.sensmetry.sysand.Sysand.defaultEnvName());
            com.sensmetry.sysand.Sysand.env(envPath);

            assertTrue(Files.exists(envPath.resolve("entries.txt")), "Entries file should exist");
            // String entries = java.nio.file.Files.readString(envPath.resolve("entries.txt"));
            String entries = new String(Files.readAllBytes(envPath.resolve("entries.txt")));
            assertEquals("", entries);
        } catch (java.io.IOException e) {
            fail("Failed during temporary directory operations or Sysand.env: " + e.getMessage());
        } catch (com.sensmetry.sysand.exceptions.SysandException e) {
            fail("Failed during temporary directory operations or Sysand.env: " + e.getMessage());
        }
    }

    private void assertExpectedProject(com.sensmetry.sysand.model.InterchangeProject project) {
        assertNotNull(project);
        assertNotNull(project.info);
        assertNotNull(project.metadata);
        assertEquals(project.info.getName(), "test_basic_info");
        assertEquals(project.info.getDescription(), null);
        assertEquals(project.info.getVersion(), "1.2.3");
        assertEquals(project.info.getLicense(), null);
        assertEquals(project.info.getMaintainer().length, 0);
        assertEquals(project.info.getWebsite(), null);
        assertEquals(project.info.getTopic().length, 0);
        assertEquals(project.info.getUsage().length, 0);

        assertEquals(project.metadata.getIndex(), new java.util.HashMap<String, String>());
        assertNotNull(project.metadata.getCreated());
        assertTrue(project.metadata.getCreated().matches("\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}.\\d{6,9}Z"));
        assertEquals(project.metadata.getMetamodel(), null);
        assertEquals(project.metadata.getIncludesDerived(), null);
        assertEquals(project.metadata.getIncludesImplied(), null);
        assertEquals(project.metadata.getChecksum(), null);
    }

    @Test
    public void testBasicInfo() {
        try {
            java.nio.file.Path tempDir = java.nio.file.Files.createTempDirectory("sysand-test-info");
            com.sensmetry.sysand.Sysand.init("test_basic_info", "1.2.3", tempDir);

            com.sensmetry.sysand.model.InterchangeProject project = com.sensmetry.sysand.Sysand.infoPath(tempDir);
            assertExpectedProject(project);

            java.net.URI fileUri = tempDir.toUri();
            com.sensmetry.sysand.model.InterchangeProject[] projects = com.sensmetry.sysand.Sysand.info(fileUri,
                    tempDir);
            assertEquals(projects.length, 1);
            assertExpectedProject(projects[0]);

            com.sensmetry.sysand.model.InterchangeProject[] projects2 = com.sensmetry.sysand.Sysand.info(fileUri);
            assertEquals(projects2.length, 1);
            assertExpectedProject(projects2[0]);
        } catch (java.io.IOException e) {
            fail("Failed during temporary directory operations or Sysand.info: " + e.getMessage());
        } catch (com.sensmetry.sysand.exceptions.SysandException e) {
            fail("Failed during temporary directory operations or Sysand.info: " + e.getMessage());
        }
    }

    @Test
    public void testHttpInfo() {
        // TODO: Find a good mock server so that we can test this.
    }

}
