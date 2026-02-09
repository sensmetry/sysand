// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand;

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
    String osName;
    String archName;
    if (os.contains("mac") || os.contains("darwin")) {
      ext = ".dylib";
      osName = "macos";
    } else if (os.contains("win")) {
      ext = ".dll";
      osName = "windows";
    } else {
      ext = ".so";
      osName = "linux";
    }
    String arch = System.getProperty("os.arch").toLowerCase(Locale.ROOT);
    if (arch.contains("arm") || arch.contains("aarch64")) {
      archName = "arm64";
    } else {
      archName = "x86_64";
    }

    String[] candidates =
        new String[] {
          "lib" + baseName + ext,
          baseName + ext,
          osName + "-" + archName + "/" + "lib" + baseName + ext,
          osName + "-" + archName + "/" + baseName + ext,
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

  private static void loadFromResources(String resourceFilePath) {
    ClassLoader cl = NativeLoader.class.getClassLoader();
    try (InputStream in = cl.getResourceAsStream(resourceFilePath)) {
      if (in == null) {
        throw new UnsatisfiedLinkError("Native library resource not found2: " + resourceFilePath);
      }

      int pathSeparatorIndex = resourceFilePath.lastIndexOf('/');
      String resourceFileName;
      if (pathSeparatorIndex != -1 && pathSeparatorIndex < resourceFilePath.length() - 1) {
        resourceFileName = resourceFilePath.substring(pathSeparatorIndex + 1);
      } else {
        resourceFileName = resourceFilePath;
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
      } catch (SecurityException ignored) {
      }

      System.load(temp.toAbsolutePath().toString());
    } catch (IOException io) {
      throw new UnsatisfiedLinkError(
          "Failed to extract and load native library: " + io.getMessage());
    }
  }
}
