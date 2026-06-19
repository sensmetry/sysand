// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand;

import org.junit.jupiter.api.Test;

import com.sensmetry.sysand.model.InterchangeProject;
import com.sensmetry.sysand.model.InterchangeProjectChecksum;
import com.sensmetry.sysand.model.InterchangeProjectInfo;
import com.sensmetry.sysand.model.InterchangeProjectMetadata;
import com.sensmetry.sysand.model.InterchangeProjectUsage;
import com.sensmetry.sysand.model.InterchangeProjectUsageDirectory;
import com.sensmetry.sysand.model.InterchangeProjectUsageKparPath;
import com.sensmetry.sysand.model.InterchangeProjectUsageResource;

import static org.junit.jupiter.api.Assertions.*;

import java.util.LinkedHashMap;

public class ModelRoundtripTest {

    @Test
    public void testModelTypesRoundtripThroughRust() {
        InterchangeProjectInfo info = new InterchangeProjectInfo(
                "roundtrip-project",
                "acme",
                "A project with every exposed info field",
                "1.2.3",
                "MIT",
                new String[]{"Alice", "Bob"},
                "https://example.com/roundtrip",
                new String[]{"sysml", "bindings"},
                new InterchangeProjectUsage[]{
                    new InterchangeProjectUsageResource("pkg:sysand/acme/remote-lib", ">=1.0.0"),
                    new InterchangeProjectUsageDirectory("../local-lib", "local-pub", "local-lib"),
                    new InterchangeProjectUsageKparPath("deps/archive-lib.kpar", "archive-pub", "archive-lib"),
                });

        LinkedHashMap<String, String> index = new LinkedHashMap<>();
        index.put("Alpha", "src/Alpha.sysml");
        index.put("Beta", "src/nested/Beta.kerml");

        LinkedHashMap<String, InterchangeProjectChecksum> checksum = new LinkedHashMap<>();
        checksum.put("src/Alpha.sysml", new InterchangeProjectChecksum("0123456789abcdef", "MD5"));
        checksum.put("src/nested/Beta.kerml", new InterchangeProjectChecksum("deadbeef", "ADLER32"));

        InterchangeProjectMetadata metadata = new InterchangeProjectMetadata(
                index,
                "2026-01-02T03:04:05Z",
                "https://www.omg.org/spec/SysML/20250201",
                Boolean.TRUE,
                Boolean.FALSE,
                checksum);

        InterchangeProject roundtripped = SysandTestHooks.modelRoundtrip(info, metadata);

        assertInfoEquals(info, roundtripped.info);
        assertMetadataEquals(metadata, roundtripped.metadata);
    }

    private static void assertInfoEquals(InterchangeProjectInfo expected, InterchangeProjectInfo actual) {
        assertEquals(expected.getName(), actual.getName());
        assertEquals(expected.getPublisher(), actual.getPublisher());
        assertEquals(expected.getDescription(), actual.getDescription());
        assertEquals(expected.getVersion(), actual.getVersion());
        assertEquals(expected.getLicense(), actual.getLicense());
        assertArrayEquals(expected.getMaintainer(), actual.getMaintainer());
        assertEquals(expected.getWebsite(), actual.getWebsite());
        assertArrayEquals(expected.getTopic(), actual.getTopic());

        InterchangeProjectUsage[] expectedUsage = expected.getUsage();
        InterchangeProjectUsage[] actualUsage = actual.getUsage();
        assertEquals(expectedUsage.length, actualUsage.length);

        InterchangeProjectUsageResource expectedResource = (InterchangeProjectUsageResource) expectedUsage[0];
        assertInstanceOf(InterchangeProjectUsageResource.class, actualUsage[0]);
        InterchangeProjectUsageResource actualResource = (InterchangeProjectUsageResource) actualUsage[0];
        assertEquals(expectedResource.getResource(), actualResource.getResource());
        assertEquals(expectedResource.getVersionConstraint(), actualResource.getVersionConstraint());

        InterchangeProjectUsageDirectory expectedDirectory = (InterchangeProjectUsageDirectory) expectedUsage[1];
        assertInstanceOf(InterchangeProjectUsageDirectory.class, actualUsage[1]);
        InterchangeProjectUsageDirectory actualDirectory = (InterchangeProjectUsageDirectory) actualUsage[1];
        assertEquals(expectedDirectory.getDirectory(), actualDirectory.getDirectory());
        assertEquals(expectedDirectory.getPublisher(), actualDirectory.getPublisher());
        assertEquals(expectedDirectory.getName(), actualDirectory.getName());

        InterchangeProjectUsageKparPath expectedKparPath = (InterchangeProjectUsageKparPath) expectedUsage[2];
        assertInstanceOf(InterchangeProjectUsageKparPath.class, actualUsage[2]);
        InterchangeProjectUsageKparPath actualKparPath = (InterchangeProjectUsageKparPath) actualUsage[2];
        assertEquals(expectedKparPath.getKparPath(), actualKparPath.getKparPath());
        assertEquals(expectedKparPath.getPublisher(), actualKparPath.getPublisher());
        assertEquals(expectedKparPath.getName(), actualKparPath.getName());
    }

    private static void assertMetadataEquals(
            InterchangeProjectMetadata expected,
            InterchangeProjectMetadata actual) {
        assertEquals(expected.getIndex(), actual.getIndex());
        assertEquals(expected.getCreated(), actual.getCreated());
        assertEquals(expected.getMetamodel(), actual.getMetamodel());
        assertEquals(expected.getIncludesDerived(), actual.getIncludesDerived());
        assertEquals(expected.getIncludesImplied(), actual.getIncludesImplied());

        LinkedHashMap<String, InterchangeProjectChecksum> expectedChecksum = expected.getChecksum();
        LinkedHashMap<String, InterchangeProjectChecksum> actualChecksum = actual.getChecksum();
        assertNotNull(actualChecksum);
        assertEquals(expectedChecksum.keySet(), actualChecksum.keySet());
        for (String key : expectedChecksum.keySet()) {
            assertEquals(expectedChecksum.get(key).getValue(), actualChecksum.get(key).getValue());
            assertEquals(expectedChecksum.get(key).getAlgorithm(), actualChecksum.get(key).getAlgorithm());
        }
    }
}
