// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::io::Read;
use std::str::FromStr;

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;
use typed_path::Utf8UnixPathBuf;

#[cfg(feature = "filesystem")]
use sysand_core::project::local_src::LocalSrcProject;
use sysand_core::project::{ProjectRead, memory::InMemoryProject};

// Currently these need to be in scope for derive(ProjectRead) to work
// TODO: Include these types in ProjectRead trait
use sysand_core::lock::Source;
use sysand_core::model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw};

#[derive(ProjectRead)]
enum SingleVariantEnum {
    Memory(InMemoryProject),
}

#[cfg(feature = "filesystem")]
#[derive(ProjectRead)]
enum DoubleVariantEnum {
    Memory(InMemoryProject),
    LocalSrc(LocalSrcProject),
}

#[test]
fn test_macro_single() {
    let _test_single = SingleVariantEnum::Memory(InMemoryProject::new());
}

#[cfg(feature = "filesystem")]
#[test]
fn test_macro_double() {
    let _test_double = DoubleVariantEnum::LocalSrc(LocalSrcProject {
        project_path: Utf8PathBuf::new(),
    });
}

#[test]
fn test_error_display_single() {
    let error = <SingleVariantEnum as ProjectRead>::Error::Memory(
        <InMemoryProject as ProjectRead>::Error::AlreadyExists("file".to_string()),
    );
    let _display = format!("{}", error);
}

#[cfg(feature = "filesystem")]
#[test]
fn test_error_display_double() {
    let error = <DoubleVariantEnum as ProjectRead>::Error::LocalSrc(
        <LocalSrcProject as ProjectRead>::Error::AlreadyExists("file".to_string()),
    );
    let _display = format!("{}", error);
}

#[cfg(feature = "filesystem")]
#[test]
fn test_macro_double_get_project() {
    let info = InterchangeProjectInfoRaw {
        name: "single_get_info".to_string(),
        description: None,
        version: "1.2.3".to_string(),
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    };
    let meta = InterchangeProjectMetadataRaw {
        index: indexmap::IndexMap::new(),
        created: "0000-00-00T00:00:00.123456789Z".to_string(),
        metamodel: None,
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    };
    let test_double = DoubleVariantEnum::Memory(InMemoryProject {
        info: Some(info.clone()),
        meta: Some(meta.clone()),
        files: HashMap::new(),
        nominal_sources: vec![],
    });

    assert_eq!(test_double.get_project().unwrap(), (Some(info), Some(meta)));
}

#[test]
fn test_macro_single_get_info() {
    let info = InterchangeProjectInfoRaw {
        name: "single_get_info".to_string(),
        description: None,
        version: "1.2.3".to_string(),
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    };
    let test_single = SingleVariantEnum::Memory(InMemoryProject {
        info: Some(info.clone()),
        meta: None,
        files: HashMap::new(),
        nominal_sources: vec![],
    });

    assert_eq!(test_single.get_info().unwrap().unwrap(), info);
}

#[test]
fn test_macro_single_get_meta() {
    let meta = InterchangeProjectMetadataRaw {
        index: indexmap::IndexMap::new(),
        created: "0000-00-00T00:00:00.123456789Z".to_string(),
        metamodel: None,
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    };
    let test_single = SingleVariantEnum::Memory(InMemoryProject {
        info: None,
        meta: Some(meta.clone()),
        files: HashMap::new(),
        nominal_sources: vec![],
    });

    assert_eq!(test_single.get_meta().unwrap().unwrap(), meta);
}

#[test]
fn test_macro_single_read_source() {
    let mut map = HashMap::new();
    let path = Utf8UnixPathBuf::from_str("path").unwrap();
    let file_content = "file content".to_string();
    map.insert(path.clone(), file_content.clone());
    let test_single = SingleVariantEnum::Memory(InMemoryProject {
        info: None,
        meta: None,
        files: map,
        nominal_sources: vec![],
    });

    let mut buffer = String::new();

    test_single
        .read_source(path)
        .unwrap()
        .read_to_string(&mut buffer)
        .unwrap();

    assert_eq!(buffer, file_content);
}

#[test]
fn test_macro_single_sources() {
    let test_single = SingleVariantEnum::Memory(InMemoryProject::new());

    assert_eq!(test_single.sources(), vec![]);
}
