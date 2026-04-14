// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

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
            InvalidWorkspace => (),
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
            3 => Ok(InvalidWorkspace),
            4 => Ok(InvalidSemanticVersion),
            5 => Ok(InvalidSPDXLicense),
            6 => Ok(InvalidValue),
            7 => Ok(SerializationError),
            8 => Ok(ResolutionError),
            9 => Ok(SysandException),
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
