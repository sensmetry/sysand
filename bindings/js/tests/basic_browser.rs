// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#![expect(clippy::tests_outside_test_module)]

#[cfg(feature = "browser")]
mod browser_tests {
    use std::error::Error;

    use sysand_core::project::{ProjectDefinition, ProjectRead as _};

    use sysand_js::{
        do_env_js_local_storage, do_init_js_local_storage,
        io::local_storage::open_project_local_storage,
    };

    use typed_path::Utf8UnixPath;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test(unsupported = test)]
    fn basic_init() -> Result<(), Box<dyn Error>> {
        do_init_js_local_storage(
            "basic-init".to_owned(),
            "acme".into(),
            "1.2.3".to_owned(),
            "sysand_storage",
            "/",
            Some("MIT OR Apache-2.0".to_owned()),
            "https://www.omg.org/spec/SysML/20250201".to_owned(),
        )
        .unwrap();

        let local_storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();

        // `init` writes a `model.toml` here as everywhere else; the two
        // json documents are what a build produces and what a legacy
        // project has.
        assert_eq!(
            local_storage.get_item("sysand_storage/model.toml").unwrap(),
            Some(
                "manifest-version = \"1.0\"\n\n\
                 [project]\n\
                 publisher = \"acme\"\n\
                 name = \"basic-init\"\n\
                 version = \"1.2.3\"\n\
                 sources = []\n\
                 license = \"MIT OR Apache-2.0\"\n\
                 metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
                    .to_owned()
            )
        );
        assert_eq!(
            local_storage
                .get_item("sysand_storage/.project.json")
                .unwrap(),
            None
        );

        let project = open_project_local_storage("sysand_storage", Utf8UnixPath::new("/"))?;

        let Some(ProjectDefinition::Toml(manifest)) = project.get_project()? else {
            return Err("Failed".into());
        };

        assert_eq!(manifest.name.as_str(), "basic-init");
        assert_eq!(manifest.publisher.as_str(), "acme");
        assert_eq!(manifest.version.to_string(), "1.2.3");
        assert_eq!(
            manifest.license.as_ref().map(ToString::to_string),
            Some("MIT OR Apache-2.0".to_owned())
        );
        assert!(manifest.sources().expect("an OMG project").is_empty());
        assert_eq!(
            manifest.usages,
            [] as [sysand_core::manifest::usage::LocatedUsage; 0]
        );

        // Local storage is not automatically cleared between tests.
        local_storage.clear().unwrap();

        Ok(())
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn basic_env() {
        do_env_js_local_storage("sysand_storage", "/").unwrap();

        let local_storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();

        assert_eq!(
            local_storage.key(0),
            Ok(Some("sysand_storage/.sysand/entries.txt".to_owned()))
        );
        assert_eq!(local_storage.key(1), Ok(None));

        assert_eq!(
            local_storage
                .get_item("sysand_storage/.sysand/entries.txt")
                .unwrap(),
            Some(String::new())
        );

        // Local storage is not automatically cleared between tests.
        local_storage.clear().unwrap();
    }
}
