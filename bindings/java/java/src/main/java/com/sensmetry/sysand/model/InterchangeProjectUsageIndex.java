// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand.model;

/**
 * The project {@code publisher}/{@code name}, found by index or any other
 * source that finds a project by its publisher and name. Both are spelled
 * exactly as the project spells them.
 */
public class InterchangeProjectUsageIndex implements InterchangeProjectUsage {

    private String publisher;
    private String name;
    private String versionConstraint;

    public InterchangeProjectUsageIndex(String publisher, String name, String versionConstraint) {
        this.publisher = publisher;
        this.name = name;
        this.versionConstraint = versionConstraint;
    }

    public String getPublisher() {
        return publisher;
    }

    public void setPublisher(String publisher) {
        this.publisher = publisher;
    }

    public String getName() {
        return name;
    }

    public void setName(String name) {
        this.name = name;
    }

    public String getVersionConstraint() {
        return versionConstraint;
    }

    public void setVersionConstraint(String versionConstraint) {
        this.versionConstraint = versionConstraint;
    }
}
