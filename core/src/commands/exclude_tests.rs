// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use typed_path::Utf8UnixPathBuf;

use super::{ExcludeError, do_exclude};
use crate::project::{ProjectMut as _, ProjectRead as _, memory::InMemoryProject};

const SYSIDE_MANIFEST: &str = "manifest-version = \"1.0.0\"\n\n\
                               [project]\n\
                               publisher = \"acme\"\n\
                               name = \"widgets\"\n\
                               version = \"1.0.0\"\n\
                               sources = [\n    \"Base.kerml\",\n    \"Parts.sysml\",\n]\n\
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

/// The `root` is a `sources` entry by rule, so excluding it would leave
/// a manifest the next read refuses.
/// Spelled as written or with a `./`, the answer is the same; any other
/// entry still goes.
#[test]
fn excluding_the_root_file_is_refused() {
    for spelling in ["Base.kerml", "./Base.kerml"] {
        let mut project = syside_project();
        let error = do_exclude(&mut project, [Utf8UnixPathBuf::from(spelling)].into_iter())
            .expect_err("the root is not excludable");
        assert_eq!(
            error.to_string(),
            format!("`{spelling}` is the project's `root`; set another `root` before excluding it")
        );
        assert!(matches!(error, ExcludeError::RootExcluded(_)));
    }

    let mut project = syside_project();
    do_exclude(
        &mut project,
        [Utf8UnixPathBuf::from("Parts.sysml")].into_iter(),
    )
    .expect("another entry is excludable");
    let manifest = project.manifest_document().expect("a manifest").to_string();
    assert!(!manifest.contains("Parts.sysml"), "{manifest}");
    assert!(manifest.contains("root = \"Base.kerml\""), "{manifest}");
}
