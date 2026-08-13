// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::fmt::{self, Display};

use jni::{
    jni_str,
    objects::JString,
    strings::{JNIStr, JNIString},
};
use sysand_core::utils::format_err;

pub(crate) trait JniExt {
    fn throw_exception(&mut self, exception_kind: ExceptionKind, message: impl AsRef<str>);
    fn throw_stdlib_exception(
        &mut self,
        exception_kind: StdlibExceptionKind,
        message: impl AsRef<str>,
    );
    fn throw_runtime_exception(&mut self, message: impl AsRef<str>);
    fn get_str(&mut self, string: &JString, variable_name: impl Display) -> Option<String>;
}

impl JniExt for jni::Env<'_> {
    fn throw_exception(&mut self, exception_kind: ExceptionKind, message: impl AsRef<str>) {
        handle_exception_throw_result(
            self.throw_new(exception_kind.java_type(), JNIString::new(&message)),
            message,
        )
    }

    fn throw_stdlib_exception(
        &mut self,
        exception_kind: StdlibExceptionKind,
        message: impl AsRef<str>,
    ) {
        handle_exception_throw_result(
            self.throw_new(exception_kind.java_type(), JNIString::new(&message)),
            message,
        )
    }

    fn throw_runtime_exception(&mut self, message: impl AsRef<str>) {
        handle_exception_throw_result(
            self.throw_new(
                jni_str!("java.lang.RuntimeException"),
                JNIString::new(&message),
            ),
            message,
        )
    }

    fn get_str(&mut self, string: &JString, variable_name: impl Display) -> Option<String> {
        match string.mutf8_chars(self) {
            Ok(string) => Some(string.into()),
            Err(error) => {
                let message = format!("failed to read `{variable_name}`: {}", format_err(&error));
                match error {
                    jni::errors::Error::NullPtr(_) => self
                        .throw_stdlib_exception(StdlibExceptionKind::NullPointerException, message),
                    _ => self.throw_runtime_exception(message),
                }

                None
            }
        }
    }
}

/// Lists all possible exceptions to be thrown, i.e.
/// all exception types defined in
/// `java/src/main/java/org/sysand/exceptions/*.java`
#[derive(Debug, Clone, Copy)]
pub(crate) enum ExceptionKind {
    IOError,
    PathError,
    ProjectAlreadyExists,
    InvalidWorkspace,
    InvalidSemanticVersion,
    InvalidSPDXLicense,
    InvalidValue,
    SerializationError,
    ResolutionError,
    SysandException,
}

impl ExceptionKind {
    pub const fn java_type(self) -> &'static JNIStr {
        match self {
            // Any of jni_str!()/jni_sig!() with dot (java.lang.Exception) or slash (java/lang/Exception)
            // notation can be used (due to normalization in `jni` and the JVM), they will all be
            // treated as "java.lang.Exception"
            Self::IOError => jni_str!("com.sensmetry.sysand.exceptions.IOError"),
            Self::PathError => jni_str!("com.sensmetry.sysand.exceptions.PathError"),
            Self::ProjectAlreadyExists => {
                jni_str!("com.sensmetry.sysand.exceptions.ProjectAlreadyExists")
            }
            Self::InvalidWorkspace => {
                jni_str!("com.sensmetry.sysand.exceptions.InvalidWorkspace")
            }
            Self::InvalidSemanticVersion => {
                jni_str!("com.sensmetry.sysand.exceptions.InvalidSemanticVersion")
            }
            Self::InvalidSPDXLicense => {
                jni_str!("com.sensmetry.sysand.exceptions.InvalidSPDXLicense")
            }
            Self::InvalidValue => jni_str!("com.sensmetry.sysand.exceptions.InvalidValue"),
            Self::SerializationError => {
                jni_str!("com.sensmetry.sysand.exceptions.SerializationError")
            }
            Self::ResolutionError => {
                jni_str!("com.sensmetry.sysand.exceptions.ResolutionError")
            }
            Self::SysandException => {
                jni_str!("com.sensmetry.sysand.exceptions.SysandException")
            }
        }
    }
}

impl fmt::Display for ExceptionKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // derive(Debug) prints enum variant name, which is exactly what we need
        fmt::Debug::fmt(self, f)
    }
}

/// Lists all possible standard library exceptions to be thrown, i.e.
/// all exception types defined in `java.lang` package.
#[derive(Debug, Clone, Copy)]
pub(crate) enum StdlibExceptionKind {
    NullPointerException,
    UnsupportedOperationException,
}

impl StdlibExceptionKind {
    pub const fn java_type(self) -> &'static JNIStr {
        match self {
            Self::NullPointerException => jni_str!("java.lang.NullPointerException"),
            Self::UnsupportedOperationException => {
                jni_str!("java.lang.UnsupportedOperationException")
            }
        }
    }
}

impl fmt::Display for StdlibExceptionKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // derive(Debug) prints enum variant name, which is exactly what we need
        fmt::Debug::fmt(self, f)
    }
}

fn handle_exception_throw_result(
    res: Result<(), jni::errors::Error>,
    original_msg: impl AsRef<str>,
) {
    // `jni` always returns `Err` when throwing an exception, to accommodate
    // usage with `?`
    match res.unwrap_err() {
        jni::errors::Error::JavaException => (),
        // Failing to throw an exception has no recovery
        other => panic!(
            "failed to throw the exception: {}\n\
            original exception message:\n{}",
            format_err(other),
            original_msg.as_ref()
        ),
    }
}

#[cfg(test)]
#[path = "./exceptions_tests.rs"]
mod tests;
