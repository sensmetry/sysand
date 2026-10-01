// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::{
    add::{AddError, do_add},
    model::{IndexUsage, InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
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

fn resource(iri: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Resource {
        resource: iri.to_owned(),
        version_constraint: None,
    }
}

fn index(publisher: &str, name: &str, constraint: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Index(IndexUsage {
        publisher: publisher.to_owned(),
        name: name.to_owned(),
        version_constraint: constraint.to_owned(),
    })
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

    let err = do_add(&mut project, &resource("pkg:sysand/acme-labs/my.project")).unwrap_err();

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

    do_add(&mut project, &resource("pkg:sysand/acme-labs/other")).unwrap();

    assert_eq!(project.info.unwrap().usage.len(), 2);
}

#[test]
fn add_writes_an_index_usage_as_spelled() {
    let mut project = project();

    assert!(do_add(&mut project, &index("Acme Labs", "My Lib", "^1")).unwrap());

    assert_eq!(
        project.info.unwrap().usage,
        [index("Acme Labs", "My Lib", "^1")]
    );
}

#[test]
fn add_merges_an_index_usage_spelled_the_same() {
    let mut project = project_with_usage(index("Acme Labs", "My Lib", "^1"));

    assert!(!do_add(&mut project, &index("Acme Labs", "My Lib", "^1")).unwrap());
    assert!(do_add(&mut project, &index("Acme Labs", "My Lib", "<1.5")).unwrap());

    assert_eq!(
        project.info.unwrap().usage,
        [index("Acme Labs", "My Lib", "^1, <1.5")]
    );
}

#[test]
fn add_refuses_an_index_usage_spelled_differently() {
    let mut project = project_with_usage(index("Acme Labs", "My Lib", "^1"));

    let err = do_add(&mut project, &index("acme labs", "my lib", "^1")).unwrap_err();

    assert_matches!(
        err,
        AddError::IndexUsageSpelledDifferently { existing, new }
            if existing == "Acme Labs/My Lib" && new == "acme labs/my lib"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_refuses_an_index_usage_over_a_legacy_purl() {
    let mut project = project_with_usage(resource("pkg:sysand/acme-labs/my-lib"));

    let err = do_add(&mut project, &index("Acme Labs", "My Lib", "^1")).unwrap_err();

    assert_eq!(
        format_err(err),
        "`pkg:sysand/acme-labs/my-lib` is already declared as a resource usage;\n\
         remove it before adding it as an index usage"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_refuses_a_legacy_purl_over_an_index_usage() {
    let mut project = project_with_usage(index("Acme Labs", "My Lib", "^1"));

    let err = do_add(&mut project, &resource("pkg:sysand/acme-labs/my-lib")).unwrap_err();

    assert_matches!(
        err,
        AddError::DuplicateIdentifier {
            existing: "an index",
            ..
        }
    );
}
