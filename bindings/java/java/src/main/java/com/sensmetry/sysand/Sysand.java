// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand;

public class Sysand {

    static {
        NativeLoader.load("sysand");
    }

    /**
     * Initialize a new project in the specified directory. The directory must
     * already exist.
     *
     * @param name    The name of the project.
     * @param version The version of the project.
     * @param path    The path to the directory in which to initialize the project.
     */
    public static native void init(String name, String version, String path)
            throws com.sensmetry.sysand.exceptions.SysandException;

    /**
     * Initialize a new project in the specified directory. The directory must
     * already exist.
     *
     * @param name    The name of the project.
     * @param version The version of the project.
     * @param path    The path to the directory in which to initialize the project.
     */
    public static void init(String name, String version, java.nio.file.Path path)
            throws com.sensmetry.sysand.exceptions.SysandException {
        init(name, version, path.toString());
    }

    /**
     * Get the value of the constant DEFAULT_ENV_NAME, which is the default name
     * of the environment directory.
     *
     * @return The value of the constant DEFAULT_ENV_NAME.
     */
    public static native String defaultEnvName();

    /**
     * Create a local sysand_env environment for installing dependencies.
     *
     * @param path
     */
    public static native void env(String path) throws com.sensmetry.sysand.exceptions.SysandException;

    /**
     * Create a local sysand_env environment for installing dependencies.
     *
     * @param path
     */
    public static void env(java.nio.file.Path path)
            throws com.sensmetry.sysand.exceptions.SysandException {
        env(path.toString());
    }

    /**
     * Get the project information and metadata at the given path.
     *
     * @param path The path to the project.
     * @return The project information and metadata.
     */
    public static native com.sensmetry.sysand.model.InterchangeProject infoPath(String path)
            throws com.sensmetry.sysand.exceptions.SysandException;

    /**
     * Get the project information and metadata at the given path.
     *
     * @param path The path to the project.
     * @return The project information and metadata.
     */
    public static com.sensmetry.sysand.model.InterchangeProject infoPath(java.nio.file.Path path)
            throws com.sensmetry.sysand.exceptions.SysandException {
        return infoPath(path.toString());
    }

    /**
     * Get the project information and metadata at the given URI.
     *
     * @param uri              The URI of the project.
     * @param relativeFileRoot The path which should be used as the root for
     *                         relative file URIs.
     * @return The project information and metadata.
     */
    public static native com.sensmetry.sysand.model.InterchangeProject[] info(
            String uri,
            String relativeFileRoot,
            String indexUrl)
            throws com.sensmetry.sysand.exceptions.SysandException;

    /**
     * Get the project information and metadata at the given URI.
     *
     * @param uri              The URI of the project.
     * @param relativeFileRoot The path which should be used as the root for
     *                         relative file URIs.
     * @return The project information and metadata.
     */
    public static com.sensmetry.sysand.model.InterchangeProject[] info(
            java.net.URI uri,
            java.nio.file.Path relativeFileRoot,
            java.net.URL indexUrl)
            throws com.sensmetry.sysand.exceptions.SysandException {
        String indexUrlString;
        if (indexUrl != null) {
            indexUrlString = indexUrl.toString();
        } else {
            indexUrlString = null;
        }
        return info(uri.toString(), relativeFileRoot.toString(), indexUrlString);
    }

    /**
     * Get the project information and metadata at the given URI.
     *
     * @param uri              The URI of the project.
     * @param relativeFileRoot The path which should be used as the root for
     *                         relative file URIs.
     * @return The project information and metadata.
     */
    public static com.sensmetry.sysand.model.InterchangeProject[] info(
            java.net.URI uri,
            java.nio.file.Path relativeFileRoot)
            throws com.sensmetry.sysand.exceptions.SysandException {
        return info(uri, relativeFileRoot, null);
    }

    /**
     * Get the project information and metadata at the given URI. Uses the current
     * directory as the relative file root.
     *
     * @param uri The URI of the project.
     * @return The project information and metadata.
     */
    public static com.sensmetry.sysand.model.InterchangeProject[] info(java.net.URI uri)
            throws com.sensmetry.sysand.exceptions.SysandException {
        java.nio.file.Path relativeFileRoot = java.nio.file.Paths.get(".");
        return info(uri, relativeFileRoot, null);
    }

    /**
     * Build Model Project Interchange file (.kpar) from the project at the given
     * path.
     *
     * @param outputPath  The path to the output file.
     * @param projectPath The path to the project.
     */
    public static native void buildProject(String outputPath, String projectPath)
            throws com.sensmetry.sysand.exceptions.SysandException;

    /**
     * Build Model Project Interchange file (.kpar) from the project at the given
     * path.
     *
     * @param outputPath  The path to the output file.
     * @param projectPath The path to the project.
     */
    public static void buildProject(java.nio.file.Path outputPath, java.nio.file.Path projectPath)
            throws com.sensmetry.sysand.exceptions.SysandException {
        buildProject(outputPath.toString(), projectPath.toString());
    }

    /**
     * Build Model Project Interchange file (.kpar) from the workspace at the given
     * path.
     *
     * @param outputPath  The path to the output file.
     * @param workspacePath The path to the workspace.
     */
    public static native void buildWorkspace(String outputPath, String workspacePath)
            throws com.sensmetry.sysand.exceptions.SysandException;

    /**
     * Build Model Project Interchange file (.kpar) from the workspace at the given
     * path.
     *
     * @param outputPath  The path to the output file.
     * @param workspacePath The path to the workspace.
     */
    public static void buildWorkspace(java.nio.file.Path outputPath, java.nio.file.Path workspacePath)
            throws com.sensmetry.sysand.exceptions.SysandException {
        buildWorkspace(outputPath.toString(), workspacePath.toString());
    }
}
