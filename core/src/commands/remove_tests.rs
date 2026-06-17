// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    project::memory::InMemoryProject,
    remove::do_remove_guess,
};

fn project_with_usage(resource: &str) -> InMemoryProject {
    InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: "main".to_owned(),
            publisher: Some("publisher".to_owned()),
            description: None,
            version: "1.2.3".to_owned(),
            license: None,
            maintainer: vec![],
            topic: vec![],
            usage: vec![InterchangeProjectUsageRaw::Resource {
                resource: resource.to_owned(),
                version_constraint: None,
            }],
            website: None,
        }),
        ..InMemoryProject::default()
    }
}

fn project() -> InMemoryProject {
    project_with_usage("pkg:sysand/acme-labs/my.project")
}

#[test]
fn remove_accepts_normalized_sysand_shorthand() {
    let mut project = project();

    let removed = do_remove_guess(&mut project, "acme-labs/my.project".to_owned()).unwrap();

    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0],
        InterchangeProjectUsageRaw::Resource {
            resource: "pkg:sysand/acme-labs/my.project".to_string(),
            version_constraint: None
        }
    );
    assert!(project.info.unwrap().usage.is_empty());
}

#[test]
fn remove_keeps_iri_resource() {
    let mut project = project_with_usage("https://example.com/acme-labs/my.project");

    let removed = do_remove_guess(
        &mut project,
        "https://example.com/acme-labs/my.project".to_owned(),
    )
    .unwrap();

    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0],
        InterchangeProjectUsageRaw::Resource {
            resource: "https://example.com/acme-labs/my.project".to_string(),
            version_constraint: None
        }
    );
    assert!(project.info.unwrap().usage.is_empty());
}

#[test]
fn remove_rejects_non_normalized_sysand_shorthand() {
    let mut project = project();

    let err = do_remove_guess(&mut project, "Acme Labs/My.Project".to_owned()).unwrap_err();

    let err = err.to_string();
    assert!(err.contains("`Acme Labs/My.Project`"), "{err}");
    assert!(err.contains("`pkg:sysand/acme-labs/my.project`"), "{err}");
    assert_eq!(project.info.unwrap().usage.len(), 1);
}
