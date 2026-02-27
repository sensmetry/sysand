// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand.model;

public class InterchangeProject {

    public InterchangeProjectInfo info;
    public InterchangeProjectMetadata metadata;

    public InterchangeProject(
        InterchangeProjectInfo info,
        InterchangeProjectMetadata metadata
    ) {
        this.info = info;
        this.metadata = metadata;
    }
}
