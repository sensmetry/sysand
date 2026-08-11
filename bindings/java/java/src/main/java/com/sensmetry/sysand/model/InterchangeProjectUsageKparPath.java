// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand.model;

public class InterchangeProjectUsageKparPath implements InterchangeProjectUsage {

    private String kparPath;
    private String publisher;
    private String name;

    public InterchangeProjectUsageKparPath(String kparPath, String publisher, String name) {
        this.kparPath = kparPath;
        this.publisher = publisher;
        this.name = name;
    }

    public String getKparPath() {
        return kparPath;
    }

    public void setKparPath(String kparPath) {
        this.kparPath = kparPath;
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
}

