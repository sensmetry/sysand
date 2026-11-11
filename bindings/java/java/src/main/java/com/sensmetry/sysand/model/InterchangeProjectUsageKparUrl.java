// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand.model;

/**
 * The project KPAR at the IRI {@code kparUrl}. {@code publisher} is
 * {@code null} exactly for an OMG standard-library IRI.
 */
public class InterchangeProjectUsageKparUrl implements InterchangeProjectUsage {

    private String kparUrl;
    private String publisher;
    private String name;

    public InterchangeProjectUsageKparUrl(String kparUrl, String publisher, String name) {
        this.kparUrl = kparUrl;
        this.publisher = publisher;
        this.name = name;
    }

    public String getKparUrl() {
        return kparUrl;
    }

    public void setKparUrl(String kparUrl) {
        this.kparUrl = kparUrl;
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

