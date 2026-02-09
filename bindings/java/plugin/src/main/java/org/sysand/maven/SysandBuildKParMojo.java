// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package org.sysand.maven;

import org.apache.maven.plugin.AbstractMojo;
import org.apache.maven.plugin.MojoExecutionException;
import org.apache.maven.plugins.annotations.LifecyclePhase;
import org.apache.maven.plugins.annotations.Mojo;
import org.apache.maven.plugins.annotations.Parameter;

@Mojo(name = "build-kpar", defaultPhase = LifecyclePhase.PACKAGE, threadSafe = false)
public class SysandBuildKParMojo extends AbstractMojo {

  /**
   * Path to the workspace.json file. Can be configured as {@code
   * <configuration><workspacePath>...</workspacePath></configuration>} or via {@code
   * -Dsysand.workspacePath=...}.
   */
  @Parameter(property = "sysand.workspacePath", required = false)
  private String workspacePath;

  /**
   * Path to the project. Can be configured as {@code
   * <configuration><projectPath>...</projectPath></configuration>} or via {@code
   * -Dsysand.projectPath=...}.
   */
  @Parameter(property = "sysand.projectPath", required = false)
  private String projectPath;

  /**
   * Path to the output directory. Can be configured as {@code
   * <configuration><outputPath>...</outputPath></configuration>} or via {@code
   * -Dsysand.outputPath=...}.
   */
  @Parameter(property = "sysand.outputPath", required = true)
  private String outputPath;

  @Override
  public void execute() throws MojoExecutionException {
    if (projectPath == null && workspacePath == null) {
      throw new MojoExecutionException(
          "Parameter 'projectPath' or 'workspacePath' must be provided");
    }

    if (outputPath == null || outputPath.trim().isEmpty()) {
      throw new MojoExecutionException("Parameter 'outputPath' must be provided and non-empty");
    }

    try {
      if (workspacePath == null) {
        getLog().info("Invoking Sysand.buildProject on: " + projectPath + " to " + outputPath);
        com.sensmetry.sysand.Sysand.buildProject(outputPath, projectPath);
        getLog().info("Sysand.buildProject completed successfully.");
      } else {
        getLog().info("Invoking Sysand.buildWorkspace on: " + workspacePath + " to " + outputPath);
        com.sensmetry.sysand.Sysand.buildWorkspace(outputPath, workspacePath);
        getLog().info("Sysand.buildWorkspace completed successfully.");
      }
    } catch (com.sensmetry.sysand.exceptions.SysandException e) {
      throw new MojoExecutionException("Sysand.build failed: " + e.getMessage(), e);
    }
  }
}
