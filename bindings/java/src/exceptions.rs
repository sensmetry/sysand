// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use jni::JNIEnv;

pub(crate) fn throw_exception(env: &mut JNIEnv<'_>, exception_kind: &str, message: String) {
    let exception_class = env
        .find_class(format!("org/sysand/exceptions/{}", exception_kind))
        .unwrap_or_else(|_| panic!("Failed to find {} class", exception_kind));
    env.throw_new(exception_class, &message)
        .expect("Failed to throw the exception");
}
