// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand.model;

public class InterchangeProjectUsageDirectory implements InterchangeProjectUsage {

    private String directory;
    private String publisher;
    private String name;

    public InterchangeProjectUsageDirectory(String directory, String publisher, String name) {
        this.directory = directory;
        this.publisher = publisher;
        this.name = name;
    }

    public String getDirectory() {
        return directory;
    }

    public void setDirectory(String directory) {
        this.directory = directory;
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

