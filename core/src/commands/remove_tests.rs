// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw, UsageRef},
    project::memory::InMemoryProject,
    remove::do_remove,
    utils::format_err,
};
use fluent_uri::Iri;

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

fn resource(iri: &str) -> UsageRef<'_> {
    UsageRef::Resource(Iri::parse(iri).unwrap())
}

fn project() -> InMemoryProject {
    project_with_usage("pkg:sysand/acme-labs/my.project")
}

#[test]
fn remove_keeps_iri_resource() {
    let mut project = project_with_usage("https://example.com/acme-labs/my.project");

    let removed = do_remove(
        &mut project,
        resource("https://example.com/acme-labs/my.project"),
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

    let err = do_remove(&mut project, resource("pkg:sysand/acme-labs/my.project")).unwrap_err();

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

    let err = do_remove(&mut project, resource("pkg:sysand/acme-labs/my.project")).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("`pkg:sysand/acme-labs/my.project`"),
        "{message}"
    );
    assert!(
        message.contains("declared as a directory usage"),
        "{message}"
    );
    assert!(!message.contains("could not find"), "{message}");
    // Nothing was removed.
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn remove_still_reports_a_genuinely_absent_usage_as_missing() {
    let mut project = project_with_directory_usage("acme-labs", "my.project");

    let err = do_remove(&mut project, resource("pkg:sysand/acme-labs/other")).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("could not find usage for `pkg:sysand/acme-labs/other`"),
        "{message}"
    );
}

#[test]
fn remove_typed_matches_the_declared_publisher_and_name_exactly() {
    let mut project = project_with_directory_usage("Acme Labs", "My.Project");

    let removed = do_remove(&mut project, UsageRef::Typed("Acme Labs", "My.Project")).unwrap();

    assert_eq!(removed.len(), 1);
    assert_eq!(project.info.unwrap().usage, []);
}

#[test]
fn remove_typed_matches_the_normalized_publisher_and_name() {
    let mut project = project_with_directory_usage("Acme Labs", "My.Project");

    let removed = do_remove(&mut project, UsageRef::Typed("acme-labs", "my.project")).unwrap();

    assert_eq!(removed.len(), 1);
    assert_eq!(project.info.unwrap().usage, []);
}

#[test]
fn remove_typed_rejects_other_spellings_of_a_typed_usage() {
    let mut project = project_with_directory_usage("Acme Labs", "My.Project");

    let err = do_remove(&mut project, UsageRef::Typed("ACME Labs", "My.Project")).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("could not find usage for `ACME Labs/My.Project`"),
        "{message}"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn remove_typed_also_removes_the_sysand_purl_resource_usage() {
    let mut project = project();
    project
        .info
        .as_mut()
        .unwrap()
        .usage
        .push(InterchangeProjectUsageRaw::KparPath {
            kpar_path: "../my.project.kpar".to_owned(),
            publisher: "Acme Labs".to_owned(),
            name: "My.Project".to_owned(),
        });

    let removed = do_remove(&mut project, UsageRef::Typed("Acme Labs", "My.Project")).unwrap();

    assert_eq!(removed.len(), 2);
    assert_eq!(project.info.unwrap().usage, []);
}

#[test]
fn remove_typed_keeps_other_projects() {
    let mut project = project_with_directory_usage("acme-labs", "my.project");

    let err = do_remove(&mut project, UsageRef::Typed("acme-labs", "other")).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("could not find usage for `acme-labs/other`"),
        "{message}"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

fn index(publisher: &str, name: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Index {
        publisher: publisher.to_owned(),
        name: name.to_owned(),
        version_constraint: "^1".to_owned(),
    }
}

fn project_with_index_usage(publisher: &str, name: &str) -> InMemoryProject {
    let mut project = project();
    project.info.as_mut().unwrap().usage = vec![index(publisher, name)];
    project
}

#[test]
fn remove_typed_index_usage_by_exact_spelling() {
    let mut project = project_with_index_usage("Acme Labs", "My Lib");

    let removed = do_remove(&mut project, UsageRef::Typed("Acme Labs", "My Lib")).unwrap();

    assert_eq!(removed, [index("Acme Labs", "My Lib")]);
    assert_eq!(project.info.unwrap().usage, []);
}

#[test]
fn remove_typed_index_usage_by_normalized_spelling() {
    let mut project = project_with_index_usage("Acme Labs", "My Lib");

    let removed = do_remove(&mut project, UsageRef::Typed("acme-labs", "my-lib")).unwrap();

    assert_eq!(removed, [index("Acme Labs", "My Lib")]);
}

#[test]
fn remove_typed_spelled_differently_suggests_the_spelling() {
    let mut project = project_with_index_usage("Acme Labs", "My Lib");

    let err = do_remove(&mut project, UsageRef::Typed("acme labs", "my lib")).unwrap_err();

    assert_eq!(
        format_err(err),
        "could not find usage for `acme labs/my lib`; did you mean `Acme Labs/My Lib`?"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn remove_typed_removes_directory_kpar_and_index_usages() {
    let mut project = project_with_directory_usage("Acme Labs", "My Lib");
    let usages = &mut project.info.as_mut().unwrap().usage;
    usages.push(InterchangeProjectUsageRaw::KparPath {
        kpar_path: "../my-lib.kpar".to_owned(),
        publisher: "Acme Labs".to_owned(),
        name: "My Lib".to_owned(),
    });
    usages.push(index("Acme Labs", "My Lib"));
    usages.push(index("Acme Labs", "Other"));

    let removed = do_remove(&mut project, UsageRef::Typed("Acme Labs", "My Lib")).unwrap();

    assert_eq!(removed.len(), 3);
    assert_eq!(project.info.unwrap().usage, [index("Acme Labs", "Other")]);
}

#[test]
fn remove_resource_points_at_an_index_usage() {
    let mut project = project_with_index_usage("Acme Labs", "My Lib");

    let err = do_remove(&mut project, resource("pkg:sysand/acme-labs/my-lib")).unwrap_err();

    assert_eq!(
        format_err(err),
        "`pkg:sysand/acme-labs/my-lib` is declared as an index usage, not as a resource usage;\n\
         remove it with `sysand remove \"Acme Labs/My Lib\"`"
    );
}
