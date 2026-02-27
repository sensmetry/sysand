// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package com.sensmetry.sysand.model;

public class InterchangeProjectChecksum {

    private String value;
    private String algorithm;

    public InterchangeProjectChecksum(String value, String algorithm) {
        this.value = value;
        this.algorithm = algorithm;
    }

    public String getValue() {
        return value;
    }

    public String getAlgorithm() {
        return algorithm;
    }
}
