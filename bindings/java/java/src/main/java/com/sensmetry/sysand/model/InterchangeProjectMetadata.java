// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand.model;

import java.util.LinkedHashMap;

public class InterchangeProjectMetadata {

    private LinkedHashMap<String, String> index;
    private String created;
    private String metamodel;
    private Boolean includesDerived;
    private Boolean includesImplied;
    private LinkedHashMap<String, InterchangeProjectChecksum> checksum;

    public InterchangeProjectMetadata(
        LinkedHashMap<String, String> index,
        String created,
        String metamodel,
        Boolean includesDerived,
        Boolean includesImplied,
        LinkedHashMap<String, InterchangeProjectChecksum> checksum
    ) {
        this.index = index;
        this.created = created;
        this.metamodel = metamodel;
        this.includesDerived = includesDerived;
        this.includesImplied = includesImplied;
        this.checksum = checksum;
    }

    public LinkedHashMap<String, String> getIndex() {
        return index;
    }

    public void setIndex(LinkedHashMap<String, String> index) {
        this.index = index;
    }

    public String getCreated() {
        return created;
    }

    public void setCreated(String created) {
        this.created = created;
    }

    public String getMetamodel() {
        return metamodel;
    }

    public void setMetamodel(String metamodel) {
        this.metamodel = metamodel;
    }

    public Boolean getIncludesDerived() {
        return includesDerived;
    }

    public void setIncludesDerived(Boolean includesDerived) {
        this.includesDerived = includesDerived;
    }

    public Boolean getIncludesImplied() {
        return includesImplied;
    }

    public void setIncludesImplied(Boolean includesImplied) {
        this.includesImplied = includesImplied;
    }

    public LinkedHashMap<String, InterchangeProjectChecksum> getChecksum() {
        // We need to clone the map to prevent the caller from modifying the
        // internal state.
        if (checksum == null) {
            return null;
        }
        return new LinkedHashMap<>(checksum);
    }

    public void setChecksum(LinkedHashMap<String, InterchangeProjectChecksum> checksum) {
        this.checksum = checksum;
    }
}
