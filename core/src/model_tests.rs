// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use indexmap::IndexMap;

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    utils::lowercase_hex,
};

#[test]
fn str_hash_agrees_with_shell() {
    // cat <(echo -n "foobar") <(echo -n "bazbum") | sha256sum | cut -f 1 -d ' '
    // ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^_ just a fancy way to write echo -n "foobarbazbum"
    //                                              as if concatenated from two separate files
    assert_eq!(
        lowercase_hex(super::project_hash_str("foobar", "bazbum")),
        "e6e2e042d1d461877c7e79cc890af5de00f603739c17486dc1464acfc0f77797"
    );
}

#[test]
fn json_hash_agrees_with_shell() {
    let info = InterchangeProjectInfoRaw {
        name: "json_hash_agrees_with_shell".to_owned(),
        publisher: None,
        description: None,
        version: "1.2.3".to_owned(),
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    };

    let meta = InterchangeProjectMetadataRaw {
        index: IndexMap::new(),
        created: "0000-00-00T00:00:00.123456789Z".to_owned(),
        metamodel: None,
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    };

    assert_eq!(
        serde_json::to_string(&info).unwrap(),
        r#"{"name":"json_hash_agrees_with_shell","version":"1.2.3"}"#
    );
    assert_eq!(
        serde_json::to_string(&meta).unwrap(),
        r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#
    );

    // cat <(echo -n '{"name":"json_hash_agrees_with_shell","version":"1.2.3"}') <(echo -n '{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}') | sha256sum | cut -f 1 -d ' '
    assert_eq!(
        lowercase_hex(super::project_hash_raw(&info, &meta)),
        "3b08c7119d89c406de6bdfbed29566077209d295736264229ad5d2e33991b3b4"
    );
}

mod index_usage {
    use crate::{
        model::{
            IndexUsage, InterchangeProjectUsage, InterchangeProjectUsageRaw,
            InterchangeProjectValidationError,
        },
        project::utils::Identifier,
    };

    fn parse(json: &str) -> Result<InterchangeProjectUsageRaw, serde_json::Error> {
        serde_json::from_str(json)
    }

    fn index(publisher: &str, name: &str, constraint: &str) -> InterchangeProjectUsageRaw {
        InterchangeProjectUsageRaw::Index(IndexUsage {
            publisher: publisher.to_owned(),
            name: name.to_owned(),
            version_constraint: constraint.to_owned(),
        })
    }

    #[test]
    fn parses_and_round_trips() {
        let json = r#"{"publisher":"Acme Labs","name":"My Lib","versionConstraint":"^1.2"}"#;
        let usage = parse(json).unwrap();
        assert_eq!(usage, index("Acme Labs", "My Lib", "^1.2"));
        assert_eq!(serde_json::to_string(&usage).unwrap(), json);
    }

    #[test]
    fn requires_a_constraint() {
        let err = parse(r#"{"publisher":"Acme Labs","name":"My Lib"}"#).unwrap_err();
        assert!(err.is_data(), "{err}");
    }

    #[test]
    fn other_kinds_with_the_same_keys_keep_their_kind() {
        let extra = r#""publisher":"acme","name":"lib","versionConstraint":"^1""#;
        assert!(matches!(
            parse(&format!(r#"{{"resource":"pkg:sysand/acme/lib",{extra}}}"#)).unwrap(),
            InterchangeProjectUsageRaw::Resource { .. }
        ));
        assert!(matches!(
            parse(&format!(r#"{{"dir":"lib",{extra}}}"#)).unwrap(),
            InterchangeProjectUsageRaw::Directory { .. }
        ));
        assert!(matches!(
            parse(&format!(r#"{{"kparPath":"lib.kpar",{extra}}}"#)).unwrap(),
            InterchangeProjectUsageRaw::KparPath { .. }
        ));
    }

    #[test]
    fn rejects_extra_keys() {
        for key in ["index", "kpar_path", "anything"] {
            let json = format!(
                r#"{{"publisher":"acme","name":"lib","versionConstraint":"^1","{key}":"x"}}"#
            );
            assert!(parse(&json).is_err(), "{json} parsed");
        }
    }

    #[test]
    fn validates() {
        let valid = index("Acme Labs", "My.Lib", "^1.2").validate().unwrap();
        assert_eq!(
            valid,
            InterchangeProjectUsage::Index(IndexUsage {
                publisher: "Acme Labs".to_owned(),
                name: "My.Lib".to_owned(),
                version_constraint: semver::VersionReq::parse("^1.2").unwrap(),
            })
        );
        assert!(matches!(
            index("A", "My Lib", "^1").validate(),
            Err(InterchangeProjectValidationError::InvalidIndexUsagePublisher { .. })
        ));
        assert!(matches!(
            index("Acme", "my..lib", "^1").validate(),
            Err(InterchangeProjectValidationError::InvalidIndexUsageName { .. })
        ));
        assert!(matches!(
            index("Acme", "My Lib", "not a constraint").validate(),
            Err(InterchangeProjectValidationError::InvalidIndexUsageVersionConstraint { .. })
        ));
    }

    #[test]
    fn no_identifier_without_publisher() {
        assert_eq!(
            Identifier::from_unvalidated_usage(&index("", "My Lib", "^1")),
            None
        );
    }

    #[test]
    fn shares_its_identifier() {
        let usage = index("Acme Labs", "My.Lib", "^1").validate().unwrap();
        let directory = InterchangeProjectUsageRaw::Directory {
            dir: "lib".to_owned(),
            publisher: "Acme Labs".to_owned(),
            name: "My.Lib".to_owned(),
        }
        .validate()
        .unwrap();
        let identifier = Identifier::from(&usage);
        assert_eq!(identifier.as_str(), "pkg:sysand/acme-labs/my.lib");
        assert_eq!(identifier, Identifier::from(&directory));
        assert_eq!(
            Some(identifier),
            Identifier::from_unvalidated_usage(&InterchangeProjectUsageRaw::Resource {
                resource: "pkg:sysand/acme-labs/my.lib".to_owned(),
                version_constraint: None,
            })
        );
    }
}
