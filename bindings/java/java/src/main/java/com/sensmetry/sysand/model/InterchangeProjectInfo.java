// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand.model;

public class InterchangeProjectInfo {

  private String name;
  private String description;
  private String version;
  private String license;
  private String[] maintainer;
  private String website;
  private String[] topic;
  private InterchangeProjectUsage[] usage;

  public InterchangeProjectInfo(
      String name,
      String description,
      String version,
      String license,
      String[] maintainer,
      String website,
      String[] topic,
      InterchangeProjectUsage[] usage) {
    this.name = name;
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

  public String getDescription() {
    return description;
  }

  public String getVersion() {
    return version;
  }

  public String getLicense() {
    return license;
  }

  public String[] getMaintainer() {
    // We need to clone the array to prevent the caller from modifying the
    // internal state.
    return maintainer.clone();
  }

  public String getWebsite() {
    return website;
  }

  public String[] getTopic() {
    // We need to clone the array to prevent the caller from modifying the
    // internal state.
    return topic.clone();
  }

  public InterchangeProjectUsage[] getUsage() {
    // We need to clone the array to prevent the caller from modifying the
    // internal state.
    return usage.clone();
  }
}
