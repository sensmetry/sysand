// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

package org.sysand.maven;

import java.nio.file.Paths;

import org.apache.maven.plugin.AbstractMojo;
import org.apache.maven.plugin.MojoExecutionException;
import org.apache.maven.plugins.annotations.LifecyclePhase;
import org.apache.maven.plugins.annotations.Mojo;
import org.apache.maven.plugins.annotations.Parameter;

import com.sensmetry.sysand.model.CompressionMethod;

@Mojo(name = "build-kpar", defaultPhase = LifecyclePhase.PACKAGE, threadSafe = false)
public class SysandBuildKParMojo extends AbstractMojo {

    /**
     * Path to the workspace.json file. Can be configured as
     * {@code <configuration><workspacePath>...</workspacePath></configuration>}
     * or via {@code -Dsysand.workspacePath=...}.
     */
    @Parameter(property = "sysand.workspacePath", required = false)
    private String workspacePath;

    /**
     * Path to the project. Can be configured as
     * {@code <configuration><projectPath>...</projectPath></configuration>} or
     * via {@code -Dsysand.projectPath=...}.
     */
    @Parameter(property = "sysand.projectPath", required = false)
    private String projectPath;

    /**
     * Path to the output directory. Can be configured as
     * {@code <configuration><outputPath>...</outputPath></configuration>} or
     * via {@code -Dsysand.outputPath=...}.
     */
    @Parameter(property = "sysand.outputPath", required = true)
    private String outputPath;

    /**
     * KPAR compression method. Can be configured as
     * {@code <configuration><compressionMethod>...</compressionMethod></configuration>} or
     * via {@code -Dsysand.compressionMethod=...}.
     */
    @Parameter(property = "sysand.compressionMethod", required = false)
    private String compressionMethod;

    /**
     * <b>Experimental:</b> This parameter is subject to change in future releases.
     *
     * <p>Optional pre-release build tag appended to each built KPAR's version number.
     * For example, setting {@code <buildTag>42</buildTag>} turns version {@code 1.2.3}
     * into {@code 1.2.3-dev.42}. In workspace builds, {@code versionConstraint} fields
     * that exactly pin a sibling project's version are updated to include the tag as well.
     * Can be configured as
     * {@code <configuration><buildTag>...</buildTag></configuration>} or
     * via {@code -Dsysand.buildTag=...}.
     */
    @Parameter(property = "sysand.buildTag", required = false)
    private String buildTag;

    @Override
    public void execute() throws MojoExecutionException {
        if (projectPath == null && workspacePath == null) {
            throw new MojoExecutionException("Parameter 'projectPath' or 'workspacePath' must be provided");
        }

        if (outputPath == null || outputPath.trim().isEmpty()) {
            throw new MojoExecutionException("Parameter 'outputPath' must be provided and non-empty");
        }

        CompressionMethod compression = compressionMethod == null ? CompressionMethod.DEFLATED : CompressionMethod.valueOf(compressionMethod.toUpperCase());
        String effectiveBuildTag = (buildTag == null || buildTag.isEmpty()) ? null : buildTag;

        try {
            if (workspacePath == null) {
                getLog().info("Invoking Sysand.buildProject on: " + projectPath + " to " + outputPath + " with compression " + compressionMethod);
                com.sensmetry.sysand.Sysand.buildProject(Paths.get(outputPath), Paths.get(projectPath), compression, effectiveBuildTag);
                getLog().info("Sysand.buildProject completed successfully.");
            } else {
                getLog().info("Invoking Sysand.buildWorkspace on: " + workspacePath + " to " + outputPath + " with compression " + compressionMethod);
                com.sensmetry.sysand.Sysand.buildWorkspace(Paths.get(outputPath), Paths.get(workspacePath), compression, effectiveBuildTag);
                getLog().info("Sysand.buildWorkspace completed successfully.");
            }
        } catch (com.sensmetry.sysand.exceptions.SysandException e) {
            throw new MojoExecutionException("Sysand.build failed: " + e.getMessage(), e);
        }
    }

}
