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
    private String root;
    private String[] exports;

    /**
     * An OMG-shaped document: {@code index} carried, no {@code root} or
     * {@code exports}.
     */
    public InterchangeProjectMetadata(
        LinkedHashMap<String, String> index,
        String created,
        String metamodel,
        Boolean includesDerived,
        Boolean includesImplied,
        LinkedHashMap<String, InterchangeProjectChecksum> checksum
    ) {
        this(index, created, metamodel, includesDerived, includesImplied, checksum, null, null);
    }

    /**
     * Either shape. A Syside-metamodel project's document carries
     * {@code root} and {@code exports} and a null {@code index}; an
     * OMG-metamodel project's the reverse (see design/syside-metamodel.md).
     */
    public InterchangeProjectMetadata(
        LinkedHashMap<String, String> index,
        String created,
        String metamodel,
        Boolean includesDerived,
        Boolean includesImplied,
        LinkedHashMap<String, InterchangeProjectChecksum> checksum,
        String root,
        String[] exports
    ) {
        this.index = index;
        this.created = created;
        this.metamodel = metamodel;
        this.includesDerived = includesDerived;
        this.includesImplied = includesImplied;
        this.checksum = checksum;
        this.root = root;
        this.exports = exports;
    }

    public String getRoot() {
        return root;
    }

    public void setRoot(String root) {
        this.root = root;
    }

    public String[] getExports() {
        return exports;
    }

    public void setExports(String[] exports) {
        this.exports = exports;
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
