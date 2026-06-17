// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand.model;

public class InterchangeProjectUsageResource implements InterchangeProjectUsage {

    private String resource;
    private String versionConstraint;

    public InterchangeProjectUsageResource(String resource, String versionConstraint) {
        this.resource = resource;
        this.versionConstraint = versionConstraint;
    }

    public String getResource() {
        return resource;
    }

    public void setResource(String resource) {
        this.resource = resource;
    }

    public String getVersionConstraint() {
        return versionConstraint;
    }

    public void setVersionConstraint(String versionConstraint) {
        this.versionConstraint = versionConstraint;
    }
}
