// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand;

import org.junit.jupiter.api.Test;

import com.sensmetry.sysand.model.InterchangeProjectChecksum;
import com.sensmetry.sysand.model.InterchangeProjectInfo;
import com.sensmetry.sysand.model.InterchangeProjectMetadata;
import com.sensmetry.sysand.model.InterchangeProjectUsage;
import com.sensmetry.sysand.model.InterchangeProjectUsageDirectory;
import com.sensmetry.sysand.model.InterchangeProjectUsageKparPath;
import com.sensmetry.sysand.model.InterchangeProjectUsageResource;

import static org.junit.jupiter.api.Assertions.*;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;

public class SetProjectInfoTest {

    /**
     * A legacy json project, written by hand.
     *
     * <p>{@code Sysand.init} writes a {@code model.toml}, and
     * {@code setProjectInfo}/{@code setProjectMetadata}/{@code setProjectIndex}
     * write {@code .project.json}/{@code .meta.json} -- so the two do
     * not compose, and cannot.
     *
     * <p>Those three verbs are json-only by design: they refuse a
     * {@code model.toml} project outright, which
     * {@code testSetProjectVerbsRefuseATomlProject} asserts. A json fixture
     * is the only fixture they have.
     */
    private Path initProject() throws Exception {
        Path tempDir = Files.createTempDirectory("sysand-test-set-info");
        Files.write(tempDir.resolve(".project.json"),
                ("{\n  \"name\": \"original\",\n  \"publisher\": \"publisher-co\",\n"
                        + "  \"version\": \"1.0.0\"\n}\n").getBytes());
        Files.write(tempDir.resolve(".meta.json"),
                "{\n  \"index\": {},\n  \"created\": \"2026-01-01T00:00:00Z\"\n}\n".getBytes());
        return tempDir;
    }

    /**
     * The three write verbs refuse a {@code model.toml} project.
     *
     * <p>{@code setProjectIndex} reaches the same refusal by a longer
     * route -- it reads the metadata the toml view synthesises and then
     * tries to write it back.
     */
    @Test
    public void testSetProjectVerbsRefuseATomlProject() throws Exception {
        java.nio.file.Path tempDir = Files.createTempDirectory("sysand-test-set-info-toml");
        Files.write(tempDir.resolve("model.toml"),
                ("manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\n"
                        + "name = \"widgets\"\nversion = \"1.0.0\"\nsources = []\n").getBytes());

        com.sensmetry.sysand.model.InterchangeProjectInfo info =
                new com.sensmetry.sysand.model.InterchangeProjectInfo(
                        "widgets", "acme", null, "2.0.0", null, new String[0], null,
                        new String[0], new com.sensmetry.sysand.model.InterchangeProjectUsage[0]);
        assertThrows(com.sensmetry.sysand.exceptions.SysandException.class,
                () -> com.sensmetry.sysand.Sysand.setProjectInfo(tempDir, info));

        java.util.LinkedHashMap<String, String> index = new java.util.LinkedHashMap<>();
        index.put("Alpha", "src/Alpha.sysml");
        assertThrows(com.sensmetry.sysand.exceptions.SysandException.class,
                () -> com.sensmetry.sysand.Sysand.setProjectIndex(tempDir, index));
    }

    // --- setProjectInfo ---

    @Test
    public void testSetProjectInfoUpdatesVersion() throws Exception {
        Path dir = initProject();

        InterchangeProjectInfo updated = new InterchangeProjectInfo(
                "original", "publisher-co", null, "2.0.0", null,
                new String[]{}, null, new String[]{},
                new InterchangeProjectUsage[]{});
        Sysand.setProjectInfo(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals("2.0.0", project.info.getVersion());
    }

    @Test
    public void testSetProjectInfoUpdatesName() throws Exception {
        Path dir = initProject();

        InterchangeProjectInfo updated = new InterchangeProjectInfo(
                "renamed", "publisher-co", null, "1.0.0", null,
                new String[]{}, null, new String[]{},
                new InterchangeProjectUsage[]{});
        Sysand.setProjectInfo(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals("renamed", project.info.getName());
    }

    @Test
    public void testSetProjectInfoSetsOptionalFields() throws Exception {
        Path dir = initProject();

        InterchangeProjectInfo updated = new InterchangeProjectInfo(
                "original", "new-pub", "A description", "1.1.0", "MIT",
                new String[]{"Alice", "Bob"}, "https://example.com",
                new String[]{"modeling", "sysml"},
                new InterchangeProjectUsage[]{});
        Sysand.setProjectInfo(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals("new-pub", project.info.getPublisher());
        assertEquals("A description", project.info.getDescription());
        assertEquals("1.1.0", project.info.getVersion());
        assertEquals("MIT", project.info.getLicense());
        assertArrayEquals(new String[]{"Alice", "Bob"}, project.info.getMaintainer());
        assertEquals("https://example.com", project.info.getWebsite());
        assertArrayEquals(new String[]{"modeling", "sysml"}, project.info.getTopic());
    }

    @Test
    public void testSetProjectInfoSetsUsage() throws Exception {
        Path dir = initProject();

        InterchangeProjectUsage[] usages = new InterchangeProjectUsage[]{
                new InterchangeProjectUsageResource("urn:example:dep-a", ">=1.0.0"),
                new InterchangeProjectUsageResource("urn:example:dep-b", null),
        };
        InterchangeProjectInfo updated = new InterchangeProjectInfo(
                "original", "publisher-co", null, "1.0.0", null,
                new String[]{}, null, new String[]{}, usages);
        Sysand.setProjectInfo(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals(2, project.info.getUsage().length);
        InterchangeProjectUsageResource u1 = (InterchangeProjectUsageResource)project.info.getUsage()[0];
        assertEquals("urn:example:dep-a", u1.getResource());
        assertEquals(">=1.0.0", u1.getVersionConstraint());
        InterchangeProjectUsageResource u2 = (InterchangeProjectUsageResource)project.info.getUsage()[1];
        assertEquals("urn:example:dep-b", u2.getResource());
        assertNull(u2.getVersionConstraint());
    }

    @Test
    public void testSetProjectInfoOverwritesExisting() throws Exception {
        Path dir = initProject();

        // First update
        InterchangeProjectInfo first = new InterchangeProjectInfo(
                "first", "publisher-co", null, "1.0.0", null,
                new String[]{}, null, new String[]{},
                new InterchangeProjectUsage[]{});
        Sysand.setProjectInfo(dir, first);

        // Second update overwrites
        InterchangeProjectInfo second = new InterchangeProjectInfo(
                "second", "publisher-co", null, "2.0.0", null,
                new String[]{}, null, new String[]{},
                new InterchangeProjectUsage[]{});
        Sysand.setProjectInfo(dir, second);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals("second", project.info.getName());
        assertEquals("2.0.0", project.info.getVersion());
    }

    @Test
    public void testSetProjectInfoPersistsToFile() throws Exception {
        Path dir = initProject();

        InterchangeProjectInfo updated = new InterchangeProjectInfo(
                "persisted", "publisher-co", null, "3.0.0", "Apache-2.0",
                new String[]{}, null, new String[]{},
                new InterchangeProjectUsage[]{});
        Sysand.setProjectInfo(dir, updated);

        String json = new String(Files.readAllBytes(dir.resolve(".project.json")));
        assertTrue(json.contains("\"name\": \"persisted\""));
        assertTrue(json.contains("\"version\": \"3.0.0\""));
        assertTrue(json.contains("\"license\": \"Apache-2.0\""));
    }

    @Test
    public void testSetProjectInfoDoesNotTouchMetadata() throws Exception {
        Path dir = initProject();

        // Set an index so we have something to verify is untouched
        LinkedHashMap<String, String> index = new LinkedHashMap<>();
        index.put("Foo", "src/Foo.sysml");
        Sysand.setProjectIndex(dir, index);

        InterchangeProjectInfo updated = new InterchangeProjectInfo(
                "original", "publisher-co", null, "9.9.9", null,
                new String[]{}, null, new String[]{},
                new InterchangeProjectUsage[]{});
        Sysand.setProjectInfo(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals("9.9.9", project.info.getVersion());
        assertEquals(1, project.metadata.getIndex().size());
        assertEquals("src/Foo.sysml", project.metadata.getIndex().get("Foo"));
    }

    @Test
    public void testSetProjectInfoWithDirectoryUsage() throws Exception {
        Path dir = initProject();

        InterchangeProjectUsageDirectory usage = new InterchangeProjectUsageDirectory(
                "../some-dep", "acme", "lib");
        InterchangeProjectInfo updated = new InterchangeProjectInfo(
                "original", "publisher-co", null, "1.0.0", null,
                new String[]{}, null, new String[]{},
                new InterchangeProjectUsage[]{usage});

        Sysand.setProjectInfo(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals(1, project.info.getUsage().length);
        assertInstanceOf(InterchangeProjectUsageDirectory.class, project.info.getUsage()[0]);
        InterchangeProjectUsageDirectory d =
                (InterchangeProjectUsageDirectory) project.info.getUsage()[0];
        assertEquals("../some-dep", d.getDirectory());
        assertEquals("acme", d.getPublisher());
        assertEquals("lib", d.getName());
    }

    @Test
    public void testSetProjectInfoWithKparPathUsage() throws Exception {
        Path dir = initProject();

        InterchangeProjectUsageKparPath usage = new InterchangeProjectUsageKparPath(
                "deps/lib.kpar", "acme", "lib");
        InterchangeProjectInfo updated = new InterchangeProjectInfo(
                "original", "publisher-co", null, "1.0.0", null,
                new String[]{}, null, new String[]{},
                new InterchangeProjectUsage[]{usage});

        Sysand.setProjectInfo(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals(1, project.info.getUsage().length);
        assertInstanceOf(InterchangeProjectUsageKparPath.class, project.info.getUsage()[0]);
        InterchangeProjectUsageKparPath k =
                (InterchangeProjectUsageKparPath) project.info.getUsage()[0];
        assertEquals("deps/lib.kpar", k.getKparPath());
        assertEquals("acme", k.getPublisher());
        assertEquals("lib", k.getName());
    }

    // --- setProjectMetadata ---

    @Test
    public void testSetProjectMetadataUpdatesIndex() throws Exception {
        Path dir = initProject();

        LinkedHashMap<String, String> index = new LinkedHashMap<>();
        index.put("Alpha", "src/Alpha.sysml");
        index.put("Beta", "src/Beta.sysml");

        com.sensmetry.sysand.model.InterchangeProject existing = Sysand.infoPath(dir);
        InterchangeProjectMetadata updated = new InterchangeProjectMetadata(
                index,
                existing.metadata.getCreated(),
                null, null, null, null);
        Sysand.setProjectMetadata(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals(2, project.metadata.getIndex().size());
        assertEquals("src/Alpha.sysml", project.metadata.getIndex().get("Alpha"));
        assertEquals("src/Beta.sysml", project.metadata.getIndex().get("Beta"));
    }

    @Test
    public void testSetProjectMetadataSetsOptionalFields() throws Exception {
        Path dir = initProject();

        LinkedHashMap<String, String> index = new LinkedHashMap<>();
        com.sensmetry.sysand.model.InterchangeProject existing = Sysand.infoPath(dir);
        InterchangeProjectMetadata updated = new InterchangeProjectMetadata(
                index,
                existing.metadata.getCreated(),
                "urn:example:metamodel",
                true,
                false,
                null);
        Sysand.setProjectMetadata(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals("urn:example:metamodel", project.metadata.getMetamodel());
        assertEquals(Boolean.TRUE, project.metadata.getIncludesDerived());
        assertEquals(Boolean.FALSE, project.metadata.getIncludesImplied());
    }

    @Test
    public void testSetProjectMetadataSetsChecksum() throws Exception {
        Path dir = initProject();

        LinkedHashMap<String, InterchangeProjectChecksum> checksum = new LinkedHashMap<>();
        checksum.put("src/Foo.sysml", new InterchangeProjectChecksum("abc123", "SHA-256"));

        LinkedHashMap<String, String> index = new LinkedHashMap<>();
        index.put("Foo", "src/Foo.sysml");

        com.sensmetry.sysand.model.InterchangeProject existing = Sysand.infoPath(dir);
        InterchangeProjectMetadata updated = new InterchangeProjectMetadata(
                index,
                existing.metadata.getCreated(),
                null, null, null,
                checksum);
        Sysand.setProjectMetadata(dir, updated);

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertNotNull(project.metadata.getChecksum());
        assertEquals(1, project.metadata.getChecksum().size());
        assertEquals("abc123", project.metadata.getChecksum().get("src/Foo.sysml").getValue());
        assertEquals("SHA-256", project.metadata.getChecksum().get("src/Foo.sysml").getAlgorithm());
    }

    @Test
    public void testSetProjectMetadataOverwritesExisting() throws Exception {
        Path dir = initProject();

        LinkedHashMap<String, String> firstIndex = new LinkedHashMap<>();
        firstIndex.put("Old", "src/Old.sysml");
        com.sensmetry.sysand.model.InterchangeProject existing = Sysand.infoPath(dir);
        Sysand.setProjectMetadata(dir, new InterchangeProjectMetadata(
                firstIndex, existing.metadata.getCreated(), null, null, null, null));

        LinkedHashMap<String, String> secondIndex = new LinkedHashMap<>();
        secondIndex.put("New", "src/New.sysml");
        Sysand.setProjectMetadata(dir, new InterchangeProjectMetadata(
                secondIndex, existing.metadata.getCreated(), null, null, null, null));

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals(1, project.metadata.getIndex().size());
        assertNull(project.metadata.getIndex().get("Old"));
        assertEquals("src/New.sysml", project.metadata.getIndex().get("New"));
    }

    @Test
    public void testSetProjectMetadataPersistsToFile() throws Exception {
        Path dir = initProject();

        LinkedHashMap<String, String> index = new LinkedHashMap<>();
        index.put("Foo", "src/Foo.sysml");
        com.sensmetry.sysand.model.InterchangeProject existing = Sysand.infoPath(dir);
        Sysand.setProjectMetadata(dir, new InterchangeProjectMetadata(
                index, existing.metadata.getCreated(),
                "urn:example:mm", null, null, null));

        String json = new String(Files.readAllBytes(dir.resolve(".meta.json")));
        assertTrue(json.contains("\"Foo\": \"src/Foo.sysml\""));
        assertTrue(json.contains("\"metamodel\": \"urn:example:mm\""));
    }

    @Test
    public void testSetProjectMetadataDoesNotTouchInfo() throws Exception {
        Path dir = initProject();

        LinkedHashMap<String, String> index = new LinkedHashMap<>();
        com.sensmetry.sysand.model.InterchangeProject existing = Sysand.infoPath(dir);
        Sysand.setProjectMetadata(dir, new InterchangeProjectMetadata(
                index, existing.metadata.getCreated(), null, null, null, null));

        com.sensmetry.sysand.model.InterchangeProject project = Sysand.infoPath(dir);
        assertEquals("original", project.info.getName());
        assertEquals("1.0.0", project.info.getVersion());
    }
}
