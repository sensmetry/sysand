// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;

use crate::{
    model::InterchangeProjectUsage,
    project::ProjectFormat,
    resolve::{ResolutionInfo, UsageOrigin},
};

/// A usage built by a caller that reads no manifest is a json usage.
#[test]
fn a_usage_defaults_to_the_json_origin() {
    assert_eq!(
        ResolutionInfo::iri(Iri::parse("urn:kpar:a".to_owned()).expect("a valid IRI")).origin(),
        UsageOrigin::Json
    );
    assert_eq!(
        ResolutionInfo::new(
            InterchangeProjectUsage::Directory {
                dir: "../lib".into(),
                publisher: crate::model::ProjectPublisher::parse("acme".to_owned()).unwrap(),
                name: crate::model::ProjectName::parse("lib".to_owned()).unwrap(),
            },
            None,
        )
        .origin(),
        UsageOrigin::Json
    );
}

/// Four formats map onto two origins, and only one of the four is a
/// project a `model.toml` usage can have been read from: `Both` is the
/// ambiguity error and `Neither` a project with no manifest at all.
#[test]
fn only_a_toml_project_declares_a_toml_usage() {
    assert_eq!(
        UsageOrigin::of_format(ProjectFormat::Toml),
        UsageOrigin::Toml
    );
    for format in [
        ProjectFormat::Json,
        ProjectFormat::Both,
        ProjectFormat::Neither,
    ] {
        assert_eq!(UsageOrigin::of_format(format), UsageOrigin::Json);
    }
}
