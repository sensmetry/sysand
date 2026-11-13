// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;

use typed_path::Utf8UnixPathBuf;

use super::{ExcludeError, do_exclude};
use crate::project::{ProjectMut as _, memory::InMemoryProject};

const SYSIDE_MANIFEST: &str = "manifest-version = \"1.0\"\n\n\
                               [project]\n\
                               publisher = \"acme\"\n\
                               name = \"widgets\"\n\
                               version = \"1.0.0\"\n\
                               metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\n\
                               root = \"Base.kerml\"\n";

fn syside_project() -> InMemoryProject {
    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(&SYSIDE_MANIFEST.parse().expect("a manifest"), false)
        .expect("stores a manifest");
    project
        .files
        .insert("Base.kerml".into(), "package Base;\n".to_owned());
    project
        .files
        .insert("Parts.sysml".into(), "package Parts;\n".to_owned());
    project
}

/// A Syside `model.toml` names `root` and lists no `sources`, so there is
/// nothing to exclude from, the `root` file included.
#[test]
fn exclude_is_refused_on_a_syside_toml_project() {
    for path in ["Base.kerml", "Parts.sysml"] {
        let mut project = syside_project();
        let before = project.model_toml.clone();
        let error = do_exclude(&mut project, [Utf8UnixPathBuf::from(path)].into_iter())
            .expect_err("nothing to exclude from");
        assert_matches!(error, ExcludeError::RootOnly);
        assert_eq!(
            error.to_string(),
            "a Syside-metamodel project lists no `sources`; its files are reached from its `root`"
        );
        assert_eq!(project.model_toml, before, "nothing was written");
    }
}
