// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;

use jni::{JNIEnv, objects::JString};

pub(crate) trait JniExt {
    fn throw_exception(&mut self, exception_kind: ExceptionKind, message: impl AsRef<str>);
    fn throw_stdlib_exception(
        &mut self,
        exception_kind: StdlibExceptionKind,
        message: impl AsRef<str>,
    );
    fn throw_runtime_exception(&mut self, message: impl AsRef<str>);
    fn get_str(&mut self, string: &JString, variable_name: &str) -> Option<String>;
}

impl JniExt for JNIEnv<'_> {
    fn throw_exception(&mut self, exception_kind: ExceptionKind, message: impl AsRef<str>) {
        self.throw_new(
            format!("com/sensmetry/sysand/exceptions/{}", exception_kind),
            &message,
        )
        .expect("failed to throw the exception");
    }

    fn throw_stdlib_exception(
        &mut self,
        exception_kind: StdlibExceptionKind,
        message: impl AsRef<str>,
    ) {
        self.throw_new(format!("java/lang/{}", exception_kind), message)
            .expect("failed to throw the exception");
    }

    fn throw_runtime_exception(&mut self, message: impl AsRef<str>) {
        self.throw_new("java/lang/RuntimeException", message)
            .expect("failed to throw the exception");
    }

    fn get_str(&mut self, string: &JString, variable_name: &str) -> Option<String> {
        match self.get_string(string) {
            Ok(string) => Some(string.into()),
            Err(error) => match error {
                jni::errors::Error::WrongJValueType(expected, actual) => {
                    self.throw_stdlib_exception(
                        StdlibExceptionKind::IllegalArgumentException,
                        format!(
                            "`{}`: wrong JValue type, expected `{}`, got `{}`",
                            variable_name, expected, actual
                        ),
                    );
                    None
                }
                jni::errors::Error::InvalidCtorReturn => {
                    self.throw_stdlib_exception(
                        StdlibExceptionKind::IllegalStateException,
                        format!(
                            "`{}`: invalid constructor return (type must be void)",
                            variable_name
                        ),
                    );
                    None
                }
                jni::errors::Error::InvalidArgList(type_signature) => {
                    self.throw_stdlib_exception(
                        StdlibExceptionKind::IllegalArgumentException,
                        format!(
                            "`{}`: invalid argument list, type signature: `{}`",
                            variable_name, type_signature
                        ),
                    );
                    None
                }
                jni::errors::Error::MethodNotFound { name, sig } => {
                    self.throw_stdlib_exception(
                        StdlibExceptionKind::UnsupportedOperationException,
                        format!(
                            "`{}`: method not found: `{}` with signature `{}`",
                            variable_name, name, sig
                        ),
                    );
                    None
                }
                jni::errors::Error::FieldNotFound { name, sig } => {
                    self.throw_stdlib_exception(
                        StdlibExceptionKind::UnsupportedOperationException,
                        format!(
                            "`{}`: field not found: `{}` with signature `{}`",
                            variable_name, name, sig
                        ),
                    );
                    None
                }
                jni::errors::Error::JavaException => {
                    // A Java exception was already thrown, let it propagate
                    None
                }
                jni::errors::Error::JNIEnvMethodNotFound(method_name) => {
                    self.throw_runtime_exception(format!(
                        "`{}`: JNI environment method not found: `{}`",
                        variable_name, method_name
                    ));
                    None
                }
                jni::errors::Error::NullPtr(_) | jni::errors::Error::NullDeref(_) => {
                    self.throw_stdlib_exception(
                        StdlibExceptionKind::NullPointerException,
                        format!("`{}` is null", variable_name),
                    );
                    None
                }
                jni::errors::Error::TryLock => {
                    self.throw_stdlib_exception(
                        StdlibExceptionKind::IllegalStateException,
                        format!("`{}`: failed to acquire lock", variable_name),
                    );
                    None
                }
                jni::errors::Error::JavaVMMethodNotFound(method_name) => {
                    self.throw_runtime_exception(format!(
                        "`{}`: Java VM method not found: `{}`",
                        variable_name, method_name
                    ));
                    None
                }
                jni::errors::Error::FieldAlreadySet(field_name) => {
                    self.throw_stdlib_exception(
                        StdlibExceptionKind::IllegalStateException,
                        format!("`{}`: field already set: `{}`", variable_name, field_name),
                    );
                    None
                }
                jni::errors::Error::ThrowFailed(_) => {
                    self.throw_runtime_exception(format!("`{variable_name}`: {error}"));
                    None
                }
                jni::errors::Error::ParseFailed(string_stream_error, _) => {
                    self.throw_stdlib_exception(
                        StdlibExceptionKind::IllegalArgumentException,
                        format!("`{}`: parse failed: {}", variable_name, string_stream_error),
                    );
                    None
                }
                jni::errors::Error::JniCall(jni_error) => {
                    self.throw_runtime_exception(format!(
                        "`{}`: JNI call failed: {}",
                        variable_name, jni_error
                    ));
                    None
                }
            },
        }
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
    InvalidSPDXLicense,
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
#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub(crate) enum StdlibExceptionKind {
    NullPointerException,
    IllegalArgumentException,
    IllegalStateException,
    UnsupportedOperationException,
}

impl fmt::Display for StdlibExceptionKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // derive(Debug) prints enum variant name, which is exactly what we need
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, exists};

    use super::ExceptionKind;

    impl TryFrom<u8> for ExceptionKind {
        type Error = ();

        fn try_from(value: u8) -> Result<Self, Self::Error> {
            use super::ExceptionKind::*;

            // This is just a reminder to cover all enum variants
            match ExceptionKind::IOError {
                IOError => (),
                PathError => (),
                ProjectAlreadyExists => (),
                InvalidSemanticVersion => (),
                InvalidSPDXLicense => (),
                InvalidValue => (),
                SerializationError => (),
                ResolutionError => (),
                SysandException => (),
            }

            match value {
                0 => Ok(IOError),
                1 => Ok(PathError),
                2 => Ok(ProjectAlreadyExists),
                3 => Ok(InvalidSemanticVersion),
                4 => Ok(InvalidSPDXLicense),
                5 => Ok(InvalidValue),
                6 => Ok(SerializationError),
                7 => Ok(ResolutionError),
                8 => Ok(SysandException),
                _ => Err(()),
            }
        }
    }

    // Check that all `ExceptionKind` files exist
    // Assumes that current dir is `bindings/java/`
    #[test]
    fn test_exceptions_all_exist() {
        for kind_id in 0.. {
            match ExceptionKind::try_from(kind_id) {
                Ok(kind) => {
                    let p = format!(
                        "java/src/main/java/com/sensmetry/sysand/exceptions/{}.java",
                        kind
                    );
                    assert!(exists(&p).unwrap(), "exception `{p}` not found");
                }
                Err(_) => break,
            }
        }
    }

    // Check that all exception kinds are listed in `ExceptionKind`
    #[test]
    fn test_exceptions_all_listed() {
        let known_exceptions: Vec<String> = (0..)
            .map_while(|x| match ExceptionKind::try_from(x) {
                Ok(exc) => Some(exc.to_string()),
                Err(_) => None,
            })
            .collect();
        for file in fs::read_dir("java/src/main/java/com/sensmetry/sysand/exceptions/").unwrap() {
            let file = file.unwrap();
            let path = file.path();
            assert!(
                file.metadata().unwrap().is_file(),
                "`{}` should be a file",
                path.display(),
            );
            let de = file.file_name();
            let exception_name = de
                .to_str()
                .unwrap()
                .strip_suffix(".java")
                .expect("expected this to be java file");
            assert!(
                known_exceptions.iter().any(|x| x == exception_name),
                "exception at `{}` not listed in `ExceptionKind` enum",
                path.display()
            );
        }
    }
}
