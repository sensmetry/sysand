// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand.model;

public class InterchangeProjectUsage {

    private String resource;
    private String versionConstraint;

    public InterchangeProjectUsage(String resource, String versionConstraint) {
        this.resource = resource;
        this.versionConstraint = versionConstraint;
    }

    public String getResource() {
        return resource;
    }

    public String getVersionConstraint() {
        return versionConstraint;
    }
}
