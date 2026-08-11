// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

package com.sensmetry.sysand;

/**
 * Test-only native entry points. This class lives in the test sources, so
 * these methods are not part of the published Java API, even though their
 * implementations ship inside the native library. JNI resolves natives by
 * the declaring class name, so the Rust side exports them under
 * {@code Java_com_sensmetry_sysand_SysandTestHooks_*}.
 */
final class SysandTestHooks {

    static {
        // Trigger Sysand's static initializer, which loads the native
        // library, instead of extracting a second copy via NativeLoader.
        try {
            Class.forName(Sysand.class.getName());
        } catch (ClassNotFoundException e) {
            throw new ExceptionInInitializerError(e);
        }
    }

    private SysandTestHooks() {
    }

    /**
     * Converts the given project to the Rust model types and back, without
     * touching the filesystem. Exists purely so tests can verify that the
     * Java model classes stay in sync with the Rust definitions in
     * {@code core/src/model.rs}.
     */
    static native com.sensmetry.sysand.model.InterchangeProject modelRoundtrip(
            com.sensmetry.sysand.model.InterchangeProjectInfo info,
            com.sensmetry.sysand.model.InterchangeProjectMetadata metadata);
}
