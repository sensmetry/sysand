// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::{
    add::{do_add_guess, expand_sysand_purl_shorthand},
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    project::memory::InMemoryProject,
};
use std::assert_matches;

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
    assert_matches!(
        expand_sysand_purl_shorthand("ab/proj0"),
        Err(crate::purl::SysandPurlError::InvalidPublisher { .. })
    );
}

#[test]
fn purl_shorthand_expansion_keeps_iri_resource() {
    assert_eq!(
        expand_sysand_purl_shorthand("https://example.com/acme-labs/my.project").unwrap(),
        None
    );
}

#[test]
fn add_accepts_normalized_sysand_shorthand() {
    let mut project = project();

    do_add_guess(
        &mut project,
        "acme-labs/my.project".to_owned(),
        Some("1.2.3".to_owned()),
    )
    .unwrap();

    let info = project.info.unwrap();
    assert_eq!(info.usage.len(), 1);
    assert_eq!(
        info.usage[0],
        InterchangeProjectUsageRaw::Resource {
            resource: "pkg:sysand/acme-labs/my.project".to_string(),
            version_constraint: Some("^1.2.3".to_string())
        }
    );
}

#[test]
fn add_keeps_iri_resource() {
    let mut project = project();

    do_add_guess(
        &mut project,
        "https://example.com/acme-labs/my.project".to_owned(),
        None,
    )
    .unwrap();

    let info = project.info.unwrap();
    assert_eq!(info.usage.len(), 1);
    assert_eq!(
        info.usage[0],
        InterchangeProjectUsageRaw::Resource {
            resource: "https://example.com/acme-labs/my.project".to_string(),
            version_constraint: None
        }
    );
}

#[test]
fn add_rejects_non_normalized_sysand_shorthand() {
    let mut project = project();

    let err = do_add_guess(&mut project, "Acme Labs/My.Project".to_owned(), None).unwrap_err();

    let err = err.to_string();
    assert!(err.contains("`Acme Labs/My.Project`"), "{err}");
    assert!(err.contains("`pkg:sysand/acme-labs/my.project`"), "{err}");
    assert!(project.info.unwrap().usage.is_empty());
}
