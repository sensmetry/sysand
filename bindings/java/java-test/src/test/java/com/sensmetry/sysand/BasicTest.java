// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand;

import org.junit.jupiter.api.Test;

import com.sensmetry.sysand.model.CompressionMethod;

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
            com.sensmetry.sysand.Sysand.init("test", "a", "1.0.0", null, tempDir);

            // Add basic assertions to verify project creation
            assertTrue(Files.exists(tempDir.resolve(".project.json")), "Project file should exist");
            assertTrue(Files.exists(tempDir.resolve(".meta.json")), "Metadata file should exist");

            // java.nio.file.Files.readString is available in Java 11+
            // String projectJson = java.nio.file.Files.readString(tempDir.resolve(".project.json"));
            String projectJson = new String(Files.readAllBytes(tempDir.resolve(".project.json")));
            assertEquals("{\n  \"name\": \"test\",\n  \"publisher\": \"a\",\n  \"version\": \"1.0.0\",\n  \"usage\": []\n}\n", projectJson);

            // String metaJson = Files.readString(tempDir.resolve(".meta.json"));
            String metaJson = new String(Files.readAllBytes(tempDir.resolve(".meta.json")));
            Pattern regex = Pattern.compile(
                    "\\{\\s*\"index\":\\s*\\{\\},\\s*\"created\":\\s*\"\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}.\\d{6,9}Z\"\\s*\\}\n",
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
        assertEquals(project.info.getPublisher(), "a");
        assertEquals(project.info.getDescription(), null);
        assertEquals(project.info.getVersion(), "1.2.3");
        assertEquals(project.info.getLicense(), "MIT");
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
            com.sensmetry.sysand.Sysand.init("test_basic_info", "a", "1.2.3", "MIT", tempDir);

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
    public void testProjectBuild() {
        try {
            java.nio.file.Path tempDir = java.nio.file.Files.createTempDirectory("sysand-test-build");
            com.sensmetry.sysand.Sysand.init("test_basic_info", "a", "1.2.3", "MIT", tempDir);

            com.sensmetry.sysand.model.InterchangeProject project = com.sensmetry.sysand.Sysand.infoPath(tempDir);
            assertExpectedProject(project);

            java.net.URI fileUri = tempDir.toUri();
            com.sensmetry.sysand.model.InterchangeProject[] projects = com.sensmetry.sysand.Sysand.info(fileUri,
                    tempDir);
            assertEquals(projects.length, 1);
            assertExpectedProject(projects[0]);

            com.sensmetry.sysand.Sysand.buildProject(tempDir.resolve("sysand-test-build.kpar"), tempDir, CompressionMethod.DEFLATED);
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

    @Test
    public void testSetProjectIndex() {
        try {
            java.nio.file.Path tempDir = java.nio.file.Files.createTempDirectory("sysand-test-update-index");
            com.sensmetry.sysand.Sysand.init("test_index", "a", "1.0.0", null, tempDir);

            java.util.LinkedHashMap<String, String> index = new java.util.LinkedHashMap<>();
            index.put("Foo", "src/Foo.sysml");
            index.put("Bar", "src/Bar.sysml");
            index.put("Baz", "src/sub/Baz.kerml");

            com.sensmetry.sysand.Sysand.setProjectIndex(tempDir, index);

            // Verify via infoPath that the index was persisted
            com.sensmetry.sysand.model.InterchangeProject project = com.sensmetry.sysand.Sysand.infoPath(tempDir);
            assertNotNull(project);
            assertNotNull(project.metadata);
            java.util.LinkedHashMap<String, String> readIndex = project.metadata.getIndex();
            assertEquals(3, readIndex.size());
            assertEquals("src/Foo.sysml", readIndex.get("Foo"));
            assertEquals("src/Bar.sysml", readIndex.get("Bar"));
            assertEquals("src/sub/Baz.kerml", readIndex.get("Baz"));

            // Verify the raw JSON contains the index entries
            String metaJson = new String(Files.readAllBytes(tempDir.resolve(".meta.json")));
            assertTrue(metaJson.contains("\"Foo\": \"src/Foo.sysml\""), "meta.json should contain Foo entry");
            assertTrue(metaJson.contains("\"Bar\": \"src/Bar.sysml\""), "meta.json should contain Bar entry");
            assertTrue(metaJson.contains("\"Baz\": \"src/sub/Baz.kerml\""), "meta.json should contain Baz entry");
        } catch (java.io.IOException e) {
            fail("Failed: " + e.getMessage());
        } catch (com.sensmetry.sysand.exceptions.SysandException e) {
            fail("Failed: " + e.getMessage());
        }
    }

    private void writeWorkspaceJson(java.nio.file.Path workspaceDir, String... projectNames) throws java.io.IOException {
        StringBuilder sb = new StringBuilder();
        sb.append("{\n  \"projects\": [\n");
        for (int i = 0; i < projectNames.length; i++) {
            sb.append("    {\"path\": \"").append(projectNames[i])
              .append("\", \"iris\": [\"urn:test:").append(projectNames[i]).append("\"]}");
            if (i < projectNames.length - 1) sb.append(",");
            sb.append("\n");
        }
        sb.append("  ]\n}\n");
        Files.write(workspaceDir.resolve(".workspace.json"), sb.toString().getBytes());
    }

    @Test
    public void testWorkspaceProjectPaths() {
        try {
            java.nio.file.Path tempDir = java.nio.file.Files.createTempDirectory("sysand-test-workspace-paths");

            // Create two project directories and init them
            java.nio.file.Path projA = tempDir.resolve("projA");
            java.nio.file.Path projB = tempDir.resolve("projB");
            Files.createDirectories(projA);
            Files.createDirectories(projB);
            com.sensmetry.sysand.Sysand.init("projA", "a", "1.0.0", null, projA);
            com.sensmetry.sysand.Sysand.init("projB", "a", "1.0.0", null, projB);

            // Write .workspace.json
            writeWorkspaceJson(tempDir, "projA", "projB");

            // Get project paths via API
            String[] paths = com.sensmetry.sysand.Sysand.workspaceProjectPaths(tempDir);
            assertEquals(2, paths.length);

            // Paths should be absolute and contain the project names
            java.util.Arrays.sort(paths);
            assertTrue(paths[0].endsWith("projA"), "First path should end with projA: " + paths[0]);
            assertTrue(paths[1].endsWith("projB"), "Second path should end with projB: " + paths[1]);
            assertTrue(java.nio.file.Paths.get(paths[0]).isAbsolute(), "Paths should be absolute");
            assertTrue(java.nio.file.Paths.get(paths[1]).isAbsolute(), "Paths should be absolute");
        } catch (java.io.IOException e) {
            fail("Failed: " + e.getMessage());
        } catch (com.sensmetry.sysand.exceptions.SysandException e) {
            fail("Failed: " + e.getMessage());
        }
    }

    @Test
    public void testSetWorkspaceProjectIndexes() {
        try {
            java.nio.file.Path tempDir = java.nio.file.Files.createTempDirectory("sysand-test-workspace-index");

            // Create two project directories and init them
            java.nio.file.Path projA = tempDir.resolve("projA");
            java.nio.file.Path projB = tempDir.resolve("projB");
            Files.createDirectories(projA);
            Files.createDirectories(projB);
            com.sensmetry.sysand.Sysand.init("projA", "a", "1.0.0", null, projA);
            com.sensmetry.sysand.Sysand.init("projB", "a", "1.0.0", null, projB);

            // Write .workspace.json
            writeWorkspaceJson(tempDir, "projA", "projB");

            // Get project paths and update each with distinct indexes
            String[] paths = com.sensmetry.sysand.Sysand.workspaceProjectPaths(tempDir);
            assertEquals(2, paths.length);

            java.util.LinkedHashMap<String, String> indexA = new java.util.LinkedHashMap<>();
            indexA.put("Alpha", "src/Alpha.sysml");
            indexA.put("Beta", "src/Beta.sysml");

            java.util.LinkedHashMap<String, String> indexB = new java.util.LinkedHashMap<>();
            indexB.put("Gamma", "lib/Gamma.kerml");

            // Update each project's index (sort paths to get deterministic assignment)
            java.util.Arrays.sort(paths);
            com.sensmetry.sysand.Sysand.setProjectIndex(java.nio.file.Paths.get(paths[0]), indexA);
            com.sensmetry.sysand.Sysand.setProjectIndex(java.nio.file.Paths.get(paths[1]), indexB);

            // Verify projA index
            com.sensmetry.sysand.model.InterchangeProject projectA = com.sensmetry.sysand.Sysand.infoPath(
                    java.nio.file.Paths.get(paths[0]));
            assertNotNull(projectA);
            assertEquals(2, projectA.metadata.getIndex().size());
            assertEquals("src/Alpha.sysml", projectA.metadata.getIndex().get("Alpha"));
            assertEquals("src/Beta.sysml", projectA.metadata.getIndex().get("Beta"));

            // Verify projB index
            com.sensmetry.sysand.model.InterchangeProject projectB = com.sensmetry.sysand.Sysand.infoPath(
                    java.nio.file.Paths.get(paths[1]));
            assertNotNull(projectB);
            assertEquals(1, projectB.metadata.getIndex().size());
            assertEquals("lib/Gamma.kerml", projectB.metadata.getIndex().get("Gamma"));

            // Verify raw JSON for each project
            String metaA = new String(Files.readAllBytes(java.nio.file.Paths.get(paths[0]).resolve(".meta.json")));
            assertTrue(metaA.contains("\"Alpha\""), "projA meta.json should contain Alpha");
            assertTrue(metaA.contains("\"Beta\""), "projA meta.json should contain Beta");
            assertFalse(metaA.contains("\"Gamma\""), "projA meta.json should not contain Gamma");

            String metaB = new String(Files.readAllBytes(java.nio.file.Paths.get(paths[1]).resolve(".meta.json")));
            assertTrue(metaB.contains("\"Gamma\""), "projB meta.json should contain Gamma");
            assertFalse(metaB.contains("\"Alpha\""), "projB meta.json should not contain Alpha");
        } catch (java.io.IOException e) {
            fail("Failed: " + e.getMessage());
        } catch (com.sensmetry.sysand.exceptions.SysandException e) {
            fail("Failed: " + e.getMessage());
        }
    }

}
