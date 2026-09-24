// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::{
    model::{IndexUsage, InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    project::memory::InMemoryProject,
    remove::{RemoveError, do_remove, do_remove_index},
    utils::format_err,
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
fn remove_keeps_iri_resource() {
    let mut project = project_with_usage("https://example.com/acme-labs/my.project");

    let removed = do_remove(
        &mut project,
        "https://example.com/acme-labs/my.project".to_owned(),
    )
    .unwrap();

    assert_eq!(removed.len(), 1);
    assert_eq!(project.info.unwrap().usage, []);
}

fn project_with_directory_usage(publisher: &str, name: &str) -> InMemoryProject {
    let mut project = project();
    project.info.as_mut().unwrap().usage = vec![InterchangeProjectUsageRaw::Directory {
        dir: "../local-lib".to_owned(),
        publisher: publisher.to_owned(),
        name: name.to_owned(),
    }];
    project
}

#[test]
fn remove_refuses_a_typed_usage_rather_than_reporting_it_missing() {
    let mut project = project_with_directory_usage("acme-labs", "my.project");

    let err = do_remove(&mut project, "pkg:sysand/acme-labs/my.project".to_owned()).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("declared as a directory usage"),
        "{message}"
    );
    assert!(!message.contains("could not find"), "{message}");
    // Nothing was removed.
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn remove_matches_a_typed_usage_through_the_normalized_identifier() {
    // `Directory` stores `publisher`/`name` unnormalized; the identifier does
    // not. Comparing the raw strings would report "not found" instead.
    let mut project = project_with_directory_usage("Acme Labs", "My.Project");

    let err = do_remove(&mut project, "pkg:sysand/acme-labs/my.project".to_owned()).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("`pkg:sysand/acme-labs/my.project`"),
        "{message}"
    );
    assert!(
        message.contains("declared as a directory usage"),
        "{message}"
    );
}

#[test]
fn remove_still_reports_a_genuinely_absent_usage_as_missing() {
    let mut project = project_with_directory_usage("acme-labs", "my.project");

    let err = do_remove(&mut project, "pkg:sysand/acme-labs/other".to_owned()).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("could not find usage for `pkg:sysand/acme-labs/other`"),
        "{message}"
    );
}

fn index(publisher: &str, name: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Index(IndexUsage {
        publisher: publisher.to_owned(),
        name: name.to_owned(),
        version_constraint: "^1".to_owned(),
    })
}

fn project_with_index_usage(publisher: &str, name: &str) -> InMemoryProject {
    let mut project = project();
    project.info.as_mut().unwrap().usage = vec![index(publisher, name)];
    project
}

#[test]
fn remove_index_by_exact_spelling() {
    let mut project = project_with_index_usage("Acme Labs", "My Lib");

    let removed = do_remove_index(&mut project, "Acme Labs", "My Lib").unwrap();

    assert_eq!(removed, [index("Acme Labs", "My Lib")]);
    assert_eq!(project.info.unwrap().usage, []);
}

#[test]
fn remove_index_spelled_differently_suggests_the_spelling() {
    let mut project = project_with_index_usage("Acme Labs", "My Lib");

    let err = do_remove_index(&mut project, "acme labs", "my lib").unwrap_err();

    assert_eq!(
        format_err(err),
        "could not find index usage `acme labs/my lib`; did you mean `Acme Labs/My Lib`?"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn remove_index_points_at_a_legacy_purl() {
    let mut project = project_with_usage("pkg:sysand/acme-labs/my-lib");

    let err = do_remove_index(&mut project, "Acme Labs", "My Lib").unwrap_err();

    assert_eq!(
        format_err(err),
        "`pkg:sysand/acme-labs/my-lib` is declared as a resource usage, not as an index usage;\n\
         remove it with `sysand remove pkg:sysand/acme-labs/my-lib`"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn remove_index_reports_a_genuinely_absent_usage_as_missing() {
    let mut project = project_with_index_usage("Acme Labs", "My Lib");

    let err = do_remove_index(&mut project, "Acme Labs", "Other").unwrap_err();

    assert!(matches!(err, RemoveError::ExpUsageNotFound { .. }), "{err}");
}

#[test]
fn remove_resource_points_at_an_index_usage() {
    let mut project = project_with_index_usage("Acme Labs", "My Lib");

    let err = do_remove(&mut project, "pkg:sysand/acme-labs/my-lib".to_owned()).unwrap_err();

    assert_eq!(
        format_err(err),
        "`pkg:sysand/acme-labs/my-lib` is declared as an index usage, not as a resource usage;\n\
         remove it with `sysand remove \"Acme Labs/My Lib\"`"
    );
}
