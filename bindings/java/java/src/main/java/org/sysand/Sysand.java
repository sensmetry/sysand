// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package org.sysand;

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
            throws org.sysand.exceptions.SysandException;

    /**
     * Initialize a new project in the specified directory. The directory must
     * already exist.
     *
     * @param name    The name of the project.
     * @param version The version of the project.
     * @param path    The path to the directory in which to initialize the project.
     */
    public static void init(String name, String version, java.nio.file.Path path)
            throws org.sysand.exceptions.SysandException {
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
    public static native void env(String path) throws org.sysand.exceptions.SysandException;

    /**
     * Create a local sysand_env environment for installing dependencies.
     *
     * @param path
     */
    public static void env(java.nio.file.Path path) throws org.sysand.exceptions.SysandException {
        env(path.toString());
    }

    /**
     * Get the project information and metadata at the given path.
     *
     * @param path The path to the project.
     * @return The project information and metadata.
     */
    public static native org.sysand.model.InterchangeProject info_path(String path)
            throws org.sysand.exceptions.SysandException;

    /**
     * Get the project information and metadata at the given path.
     *
     * @param path The path to the project.
     * @return The project information and metadata.
     */
    public static org.sysand.model.InterchangeProject info_path(java.nio.file.Path path)
            throws org.sysand.exceptions.SysandException {
        return info_path(path.toString());
    }

    /**
     * Get the project information and metadata at the given URI.
     *
     * @param uri              The URI of the project.
     * @param relativeFileRoot The path which should be used as the root for
     *                         relative file URIs.
     * @return The project information and metadata.
     */
    public static native org.sysand.model.InterchangeProject[] info(String uri, String relativeFileRoot,
            String indexUrl) throws org.sysand.exceptions.SysandException;

    /**
     * Get the project information and metadata at the given URI.
     *
     * @param uri              The URI of the project.
     * @param relativeFileRoot The path which should be used as the root for
     *                         relative file URIs.
     * @return The project information and metadata.
     */
    public static org.sysand.model.InterchangeProject[] info(java.net.URI uri,
            java.nio.file.Path relativeFileRoot, java.net.URL indexUrl)
            throws org.sysand.exceptions.SysandException {
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
    public static org.sysand.model.InterchangeProject[] info(java.net.URI uri,
            java.nio.file.Path relativeFileRoot) throws org.sysand.exceptions.SysandException {
        return info(uri, relativeFileRoot, null);
    }

    /**
     * Get the project information and metadata at the given URI. Uses the current
     * directory as the relative file root.
     *
     * @param uri The URI of the project.
     * @return The project information and metadata.
     */
    public static org.sysand.model.InterchangeProject[] info(java.net.URI uri)
            throws org.sysand.exceptions.SysandException {
        java.nio.file.Path relativeFileRoot = java.nio.file.Paths.get(".");
        return info(uri, relativeFileRoot, null);
    }

}
