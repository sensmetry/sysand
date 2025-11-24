// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(feature = "browser")]
mod browser_tests {
    use std::error::Error;

    use sysand_core::{model::InterchangeProjectInfo, project::ProjectRead};

    use sysand_js::{
        do_env_js_local_storage, do_new_js_local_storage,
        io::local_storage::open_project_local_storage,
    };

    use typed_path::Utf8UnixPath;
    use wasm_bindgen_test::wasm_bindgen_test;

    use semver::Version;

    use regex::Regex;

    #[wasm_bindgen_test(unsupported = test)]
    fn test_basic_new() -> Result<(), Box<dyn Error>> {
        do_new_js_local_storage(
            "test_basic_new".to_string(),
            "1.2.3".to_string(),
            "sysand_storage",
            "/",
            Some("MIT OR Apache-2.0".to_string()),
        )
        .unwrap();

        let local_storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();

        assert_eq!(
            local_storage
                .get_item("sysand_storage/.project.json")
                .unwrap(),
            Some(r#"{"name":"test_basic_new","version":"1.2.3","license":"MIT OR Apache-2.0","usage":[]}"#.to_string())
        );

        let re = Regex::new(r#"\{"index":\{\},"created":"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.(\d{3}|\d{6}|\d{9})Z"}"#).unwrap();

        let meta_js = local_storage
            .get_item("sysand_storage/.meta.json")
            .unwrap()
            .unwrap();

        assert!(re.is_match_at(&meta_js, 0), "unexpected: {}", meta_js);

        let project = open_project_local_storage("sysand_storage", Utf8UnixPath::new("/"))?;

        let (Some(info), Some(meta)) = project.get_project()? else {
            return Err("Failed".into());
        };

        assert_eq!(
            info,
            InterchangeProjectInfo {
                name: "test_basic_new".to_string(),
                description: None,
                version: Version::parse("1.2.3")?,
                license: Some("MIT OR Apache-2.0".to_string()),
                maintainer: vec![],
                website: None,
                topic: vec![],
                usage: vec![],
            }
            .into()
        );

        assert!(meta.checksum.is_none());
        assert!(meta.includes_derived.is_none());
        assert!(meta.includes_implied.is_none());
        assert!(meta.index.is_empty());
        assert!(meta.metamodel.is_none());

        // Local storage is not automatically cleared between tests.
        local_storage.clear().unwrap();

        Ok(())
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn test_basic_env() -> Result<(), Box<dyn Error>> {
        do_env_js_local_storage("sysand_storage", "/").unwrap();

        let local_storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();

        assert_eq!(
            local_storage.key(0),
            Ok(Some("sysand_storage/sysand_env/entries.txt".to_string()))
        );
        assert_eq!(local_storage.key(1), Ok(None));

        assert_eq!(
            local_storage
                .get_item("sysand_storage/sysand_env/entries.txt")
                .unwrap(),
            Some("".to_string())
        );

        // Local storage is not automatically cleared between tests.
        local_storage.clear().unwrap();

        Ok(())
    }
}
