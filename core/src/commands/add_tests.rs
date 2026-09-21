// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::{
    add::{do_add, do_add_guess, expand_sysand_purl_shorthand},
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    project::memory::InMemoryProject,
    utils::format_err,
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
            resource: "pkg:sysand/acme-labs/my.project".to_owned(),
            version_constraint: Some("^1.2.3".to_owned())
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
            resource: "https://example.com/acme-labs/my.project".to_owned(),
            version_constraint: None
        }
    );
}

#[test]
fn add_rejects_non_normalized_sysand_shorthand() {
    let mut project = project();

    let err = do_add_guess(&mut project, "Acme Labs/My.Project".to_owned(), None).unwrap_err();

    let err = format_err(err);
    assert!(err.contains("`Acme Labs/My.Project`"), "{err}");
    assert!(err.contains("`pkg:sysand/acme-labs/my.project`"), "{err}");
    assert_eq!(project.info.unwrap().usage, []);
}

fn project_with_usage(usage: InterchangeProjectUsageRaw) -> InMemoryProject {
    let mut project = project();
    project.info.as_mut().unwrap().usage = vec![usage];
    project
}

#[test]
fn add_refuses_a_resource_that_duplicates_a_directory_usage() {
    let mut project = project_with_usage(InterchangeProjectUsageRaw::Directory {
        dir: "../my.project".to_owned(),
        publisher: "acme-labs".to_owned(),
        name: "my.project".to_owned(),
    });

    let err = do_add_guess(&mut project, "acme-labs/my.project".to_owned(), None).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("already declared as a directory usage"),
        "{message}"
    );
    // The duplicate was not appended.
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_refuses_a_directory_that_duplicates_a_resource_usage() {
    let mut project = project_with_usage(InterchangeProjectUsageRaw::Resource {
        resource: "pkg:sysand/acme-labs/my.project".to_owned(),
        version_constraint: None,
    });

    let err = do_add(
        &mut project,
        &InterchangeProjectUsageRaw::Directory {
            dir: "../my.project".to_owned(),
            publisher: "acme-labs".to_owned(),
            name: "my.project".to_owned(),
        },
    )
    .unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("already declared as a resource usage"),
        "{message}"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_refuses_a_kpar_path_that_duplicates_a_directory_usage() {
    // Both are typed, but of different kinds, so neither merge path sees it.
    let mut project = project_with_usage(InterchangeProjectUsageRaw::Directory {
        dir: "../my.project".to_owned(),
        publisher: "acme-labs".to_owned(),
        name: "my.project".to_owned(),
    });

    let err = do_add(
        &mut project,
        &InterchangeProjectUsageRaw::KparPath {
            kpar_path: "../my.project.kpar".to_owned(),
            publisher: "acme-labs".to_owned(),
            name: "my.project".to_owned(),
        },
    )
    .unwrap_err();

    let message = format_err(err);
    assert!(message.contains("as a KPAR path usage"), "{message}");
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_allows_a_directory_usage_of_a_different_project() {
    let mut project = project_with_usage(InterchangeProjectUsageRaw::Directory {
        dir: "../my.project".to_owned(),
        publisher: "acme-labs".to_owned(),
        name: "my.project".to_owned(),
    });

    do_add_guess(&mut project, "acme-labs/other".to_owned(), None).unwrap();

    assert_eq!(project.info.unwrap().usage.len(), 2);
}
