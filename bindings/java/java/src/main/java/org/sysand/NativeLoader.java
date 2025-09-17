// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package org.sysand;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;

public final class NativeLoader {

    private NativeLoader() {}

    public static void load(String baseName) {
        String os = System.getProperty("os.name").toLowerCase(Locale.ROOT);
        String ext;
        if (os.contains("mac") || os.contains("darwin")) {
            ext = ".dylib";
        } else if (os.contains("win")) {
            ext = ".dll";
        } else {
            ext = ".so";
        }

        String[] candidates = new String[] {
            "lib" + baseName + ext,
            baseName + ext
        };

        UnsatisfiedLinkError lastError = null;
        for (String candidate : candidates) {
            try {
                loadFromResources(candidate);
                return;
            } catch (UnsatisfiedLinkError e) {
                lastError = e;
            }
        }

        // Fallback to standard lookup if bundled resource not found
        try {
            System.loadLibrary(baseName);
            return;
        } catch (UnsatisfiedLinkError e) {
            if (lastError != null) {
                throw lastError; // keep first meaningful error
            }
            throw e;
        }
    }

    private static void loadFromResources(String resourceFileName) {
        ClassLoader cl = NativeLoader.class.getClassLoader();
        try (InputStream in = cl.getResourceAsStream(resourceFileName)) {
            if (in == null) {
                throw new UnsatisfiedLinkError("Native library resource not found: " + resourceFileName);
            }

            String prefix = resourceFileName.replace('.', '_').replace('-', '_');
            String suffix = null;
            int dot = resourceFileName.lastIndexOf('.');
            if (dot != -1 && dot < resourceFileName.length() - 1) {
                prefix = resourceFileName.substring(0, dot).replace('.', '_').replace('-', '_');
                suffix = resourceFileName.substring(dot);
            }

            Path temp = Files.createTempFile(prefix + "_", suffix == null ? "" : suffix);
            Files.copy(in, temp, StandardCopyOption.REPLACE_EXISTING);
            temp.toFile().deleteOnExit();
            // Ensure executable permissions if possible
            try {
                temp.toFile().setReadable(true);
                temp.toFile().setExecutable(true);
            } catch (SecurityException ignored) {}

            System.load(temp.toAbsolutePath().toString());
        } catch (IOException io) {
            throw new UnsatisfiedLinkError("Failed to extract and load native library: " + io.getMessage());
        }
    }
}


