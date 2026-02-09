// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand.maven;

import org.apache.maven.plugin.AbstractMojo;
import org.apache.maven.plugin.MojoExecutionException;
import org.apache.maven.plugins.annotations.LifecyclePhase;
import org.apache.maven.plugins.annotations.Mojo;
import org.apache.maven.plugins.annotations.Parameter;

/** Mojo that calls {@code Sysand.info_path} during the package phase. */
@Mojo(name = "info-path", defaultPhase = LifecyclePhase.PACKAGE, threadSafe = false)
public class SysandInfoPathMojo extends AbstractMojo {

  /**
   * Path passed to {@code Sysand.info_path}. Can be configured as {@code
   * <configuration><infoPath>...</infoPath></configuration>} or via {@code -Dsysand.infoPath=...}.
   */
  @Parameter(property = "sysand.infoPath", required = true)
  private String infoPath;

  @Override
  public void execute() throws MojoExecutionException {
    if (infoPath == null || infoPath.trim().isEmpty()) {
      throw new MojoExecutionException("Parameter 'infoPath' must be provided and non-empty");
    }

    getLog().info("Invoking Sysand.info_path on: " + infoPath);
    try {
      // Call the native-backed Java API
      com.sensmetry.sysand.Sysand.infoPath(infoPath);
      getLog().info("Sysand.info_path completed successfully.");
    } catch (com.sensmetry.sysand.exceptions.SysandException e) {
      throw new MojoExecutionException("Sysand.info_path failed: " + e.getMessage(), e);
    }
  }
}
