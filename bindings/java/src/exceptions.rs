// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;

use jni::JNIEnv;

pub(crate) trait Throw {
    fn throw_exception(&mut self, exception_kind: ExceptionKind, message: String);
    fn throw_stdlib_exception(&mut self, exception_kind: StdlibExceptionKind, message: String);
}

impl Throw for JNIEnv<'_> {
    fn throw_exception(&mut self, exception_kind: ExceptionKind, message: String) {
        let exception_class = self
            .find_class(format!(
                "com/sensmetry/sysand/exceptions/{}",
                exception_kind
            ))
            .unwrap_or_else(|_| panic!("Failed to find {} class", exception_kind));
        self.throw_new(exception_class, &message)
            .expect("Failed to throw the exception");
    }

    fn throw_stdlib_exception(&mut self, exception_kind: StdlibExceptionKind, message: String) {
        let exception_class = self
            .find_class(format!("java/lang/{}", exception_kind))
            .unwrap_or_else(|_| panic!("Failed to find {} class", exception_kind));
        self.throw_new(exception_class, &message)
            .expect("Failed to throw the exception");
    }
}

/// Lists all possible exceptions to be thrown, i.e.
/// all exception types defined in
/// `java/src/main/java/org/sysand/exceptions/*.java`
#[derive(Debug)]
pub(crate) enum ExceptionKind {
    IOError,
    PathError,
    ProjectAlreadyExists,
    InvalidSemanticVersion,
    InvalidValue,
    SerializationError,
    ResolutionError,
    SysandException,
}

impl fmt::Display for ExceptionKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // derive(Debug) prints enum variant name, which is exactly what we need
        fmt::Debug::fmt(self, f)
    }
}

/// Lists all possible standard library exceptions to be thrown, i.e.
/// all exception types defined in `java.lang` package.
#[derive(Debug)]
pub(crate) enum StdlibExceptionKind {
    NullPointer,
    IllegalArgument,
    IllegalState,
    UnsupportedOperation,
    Runtime,
}

impl fmt::Display for StdlibExceptionKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // derive(Debug) prints enum variant name, which is exactly what we need
        fmt::Debug::fmt(self, f)
    }
}
