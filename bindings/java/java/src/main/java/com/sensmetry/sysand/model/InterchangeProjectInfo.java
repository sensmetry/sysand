// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand.model;

public class InterchangeProjectInfo {

    private String name;
    private String publisher;
    private String description;
    private String version;
    private String license;
    private String[] maintainer;
    private String website;
    private String[] topic;
    private InterchangeProjectUsage[] usage;

    public InterchangeProjectInfo(
        String name,
        String publisher,
        String description,
        String version,
        String license,
        String[] maintainer,
        String website,
        String[] topic,
        InterchangeProjectUsage[] usage
    ) {
        this.name = name;
        this.publisher = publisher;
        this.description = description;
        this.version = version;
        this.license = license;
        this.maintainer = maintainer;
        this.website = website;
        this.topic = topic;
        this.usage = usage;
    }

    public String getName() {
        return name;
    }

    public void setName(String name) {
        this.name = name;
    }

    public String getPublisher() {
        return publisher;
    }

    public void setPublisher(String publisher) {
        this.publisher = publisher;
    }

    public String getDescription() {
        return description;
    }

    public void setDescription(String description) {
        this.description = description;
    }

    public String getVersion() {
        return version;
    }

    public void setVersion(String version) {
        this.version = version;
    }

    public String getLicense() {
        return license;
    }

    public void setLicense(String license) {
        this.license = license;
    }

    public String[] getMaintainer() {
        // We need to clone the array to prevent the caller from modifying the
        // internal state.
        return maintainer.clone();
    }

    public void setMaintainer(String[] maintainer) {
        this.maintainer = maintainer.clone();
    }

    public String getWebsite() {
        return website;
    }

    public void setWebsite(String website) {
        this.website = website;
    }

    public String[] getTopic() {
        // We need to clone the array to prevent the caller from modifying the
        // internal state.
        return topic.clone();
    }

    public void setTopic(String[] topic) {
        this.topic = topic.clone();
    }

    public InterchangeProjectUsage[] getUsage() {
        // We need to clone the array to prevent the caller from modifying the
        // internal state.
        return usage.clone();
    }

    public void setUsage(InterchangeProjectUsage[] usage) {
        this.usage = usage.clone();
    }
}
