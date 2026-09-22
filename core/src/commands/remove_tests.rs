// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    project::memory::InMemoryProject,
    remove::do_remove_guess,
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
fn remove_accepts_normalized_sysand_shorthand() {
    let mut project = project();

    let removed = do_remove_guess(&mut project, "acme-labs/my.project".to_owned()).unwrap();

    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0],
        InterchangeProjectUsageRaw::Resource {
            resource: "pkg:sysand/acme-labs/my.project".to_owned(),
            version_constraint: None
        }
    );
    assert_eq!(project.info.unwrap().usage, []);
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
            resource: "https://example.com/acme-labs/my.project".to_owned(),
            version_constraint: None
        }
    );
    assert_eq!(project.info.unwrap().usage, []);
}

#[test]
fn remove_rejects_non_normalized_sysand_shorthand() {
    let mut project = project();

    let err = do_remove_guess(&mut project, "Acme Labs/My.Project".to_owned()).unwrap_err();

    let err = format_err(err);
    assert!(err.contains("`Acme Labs/My.Project`"), "{err}");
    assert!(err.contains("`pkg:sysand/acme-labs/my.project`"), "{err}");
    assert_eq!(project.info.unwrap().usage.len(), 1);
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

    let err = do_remove_guess(&mut project, "acme-labs/my.project".to_owned()).unwrap_err();

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

    let err = do_remove_guess(&mut project, "acme-labs/my.project".to_owned()).unwrap_err();

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

    let err = do_remove_guess(&mut project, "acme-labs/other".to_owned()).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("could not find usage for `pkg:sysand/acme-labs/other`"),
        "{message}"
    );
}

#[test]
fn remove_leaves_a_usage_it_cannot_interpret_alone() {
    let future_usage: InterchangeProjectUsageRaw = serde_json::from_value(serde_json::json!({
        "registry": "https://example.com/i",
        "publisher": "acme",
        "name": "future"
    }))
    .unwrap();
    let mut project = project_with_usage("pkg:sysand/acme/widget");
    project
        .info
        .as_mut()
        .unwrap()
        .usage
        .insert(0, future_usage.clone());

    let removed = do_remove_guess(&mut project, "acme/widget".to_owned()).unwrap();

    assert_eq!(removed.len(), 1);
    // Rewriting the manifest to remove one usage must not drop the one this
    // build cannot interpret.
    assert_eq!(project.info.unwrap().usage, vec![future_usage]);
}
