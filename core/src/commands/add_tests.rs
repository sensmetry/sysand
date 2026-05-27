// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::{
    add::{do_add, expand_sysand_purl_shorthand},
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    project::memory::InMemoryProject,
};

fn project() -> InMemoryProject {
    InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: "main".to_owned(),
            publisher: Some("publisher".to_owned()),
            description: None,
            version: "1.2.3".to_owned(),
            license: None,
            maintainer: vec![],
            topic: vec![],
            usage: vec![],
            website: None,
        }),
        ..InMemoryProject::default()
    }
}

#[test]
fn purl_shorthand_expansion_keeps_two_segment_non_purl_resource() {
    assert_eq!(
        expand_sysand_purl_shorthand("ab/proj0").unwrap(),
        "ab/proj0"
    );
}

#[test]
fn purl_shorthand_expansion_keeps_iri_resource() {
    assert_eq!(
        expand_sysand_purl_shorthand("https://example.com/acme-labs/my.project").unwrap(),
        "https://example.com/acme-labs/my.project"
    );
}

#[test]
fn add_accepts_normalized_sysand_shorthand() {
    let mut project = project();

    do_add(
        &mut project,
        &InterchangeProjectUsageRaw {
            resource: "acme-labs/my.project".to_owned(),
            version_constraint: Some("1.2.3".to_owned()),
        },
    )
    .unwrap();

    let info = project.info.unwrap();
    assert_eq!(info.usage.len(), 1);
    assert_eq!(info.usage[0].resource, "pkg:sysand/acme-labs/my.project");
    assert_eq!(info.usage[0].version_constraint.as_deref(), Some("^1.2.3"));
}

#[test]
fn add_keeps_iri_resource() {
    let mut project = project();

    do_add(
        &mut project,
        &InterchangeProjectUsageRaw {
            resource: "https://example.com/acme-labs/my.project".to_owned(),
            version_constraint: None,
        },
    )
    .unwrap();

    let info = project.info.unwrap();
    assert_eq!(info.usage.len(), 1);
    assert_eq!(
        info.usage[0].resource,
        "https://example.com/acme-labs/my.project"
    );
}

#[test]
fn add_rejects_non_normalized_sysand_shorthand() {
    let mut project = project();

    let err = do_add(
        &mut project,
        &InterchangeProjectUsageRaw {
            resource: "Acme Labs/My.Project".to_owned(),
            version_constraint: None,
        },
    )
    .unwrap_err();

    let err = err.to_string();
    assert!(err.contains("`Acme Labs/My.Project`"), "{err}");
    assert!(err.contains("`pkg:sysand/acme-labs/my.project`"), "{err}");
    assert!(project.info.unwrap().usage.is_empty());
}
