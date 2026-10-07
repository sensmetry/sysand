// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use indexmap::IndexMap;

use crate::{
    model::{
        InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, PROJECT_FIELD_MAX_LEN,
        ProjectName, ProjectPublisher,
    },
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

/// Message for a char that is not allowed, described as `$c`
macro_rules! disallowed {
    ($c:literal) => {
        concat!(
            "cannot contain ",
            $c,
            "; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed"
        )
    };
}

/// Inputs rejected by both `ProjectPublisher::parse` and `ProjectName::parse`,
/// with the expected message suffix (after `publisher `/`name `)
const INVALID_PUBLISHERS_NAMES: &[(&str, &str)] = &[
    ("", "cannot be empty"),
    ("a/b", "cannot contain `/`"),
    ("a:b", "cannot contain `:`"),
    ("a<b", "cannot contain `<`"),
    ("a>b", "cannot contain `>`"),
    (
        "a\tb",
        "cannot contain `\\t`; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    (
        "a\nb",
        "cannot contain `\\n`; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    (
        "a\0b",
        "cannot contain `\\0`; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    (
        "a\u{7f}b",
        "cannot contain `\\u{7f}`; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    (" ", "cannot start with ` `"),
    (" a", "cannot start with ` `"),
    ("a ", "cannot end with ` `"),
    (
        "a*b",
        "cannot contain `*`; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    ("\u{a0}a", disallowed!("U+00A0")),
    (
        "a\u{200b}b",
        "cannot contain U+200B; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    (
        "a\u{202e}b",
        "cannot contain U+202E; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    (
        "\u{3164}",
        "cannot contain U+3164; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    (
        "a\u{e000}",
        "cannot contain U+E000; only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    (
        "a\u{1f600}",
        "cannot contain `\u{1f600}` (U+1F600); only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    (".", "must contain at least one letter or digit"),
    ("..", "must contain at least one letter or digit"),
    ("-_.", "must contain at least one letter or digit"),
    (
        "a`b",
        "cannot contain U+0060 (backtick); only letters, digits, space, non-ASCII punctuation and `_-.&',+()` are allowed",
    ),
    ("a\u{2122}", disallowed!("`\u{2122}` (U+2122)")),
    // Fullwidth math symbol, not punctuation
    ("a\u{ff0b}b", disallowed!("`\u{ff0b}` (U+FF0B)")),
    // Abuses
    // Shell, JSON and TOML metacharacters
    ("a$b", disallowed!("`$`")),
    ("a;b", disallowed!("`;`")),
    ("a|b", disallowed!("`|`")),
    ("a?b", disallowed!("`?`")),
    ("a#b", disallowed!("`#`")),
    ("a@b", disallowed!("`@`")),
    ("a!b", disallowed!("`!`")),
    ("a%b", disallowed!("`%`")),
    ("a=b", disallowed!("`=`")),
    ("a~b", disallowed!("`~`")),
    ("a^b", disallowed!("`^`")),
    ("a[b]", disallowed!("`[`")),
    ("a{b}", disallowed!("`{`")),
    ("a\\b", disallowed!("`\\`")),
    ("a\"b", disallowed!("`\"`")),
    // Paths, IRIs and CLI options
    ("../x", "cannot contain `/`"),
    ("C:\\x", "cannot contain `:`"),
    ("https://example.com", "cannot contain `:`"),
    ("pkg:sysand/a/b", "cannot contain `:`"),
    ("--", "must contain at least one letter or digit"),
    // Invisible and format chars
    ("a\u{200d}b", disallowed!("U+200D")),
    ("a\u{fe0f}", disallowed!("U+FE0F")),
    // No letter or digit
    ("_", "must contain at least one letter or digit"),
];

const VALID_PUBLISHERS_NAMES: &[&str] = &[
    "a",
    "1",
    "1a",
    "a_",
    "a_b",
    "_a",
    "Acme Labs",
    "my.project-1_x",
    "Ąžuolas",
    "a b",
    "日本",
    "\u{915}\u{94d}\u{937}",
    "l\u{b7}l",
    "a.",
    "a-.b",
    "a - b",
    "ACME Inc.",
    "AT&T",
    "O'Reilly Media",
    "Foo, Inc.",
    "C++ Tools",
    "C++",
    "a++b",
    "Foo (EU (West))",
    "J. R. Smith",
    "a((b",
    // Fullwidth letters and digits
    "\u{ff21}\u{ff22}\u{ff23}",
    "\u{ff11}",
    // Punctuation, including non-ASCII, anywhere but the start
    "a. .b",
    "a-",
    "a,",
    "a&",
    "a(",
    "a'",
    "O\u{2019}Reilly",
    "a\u{2013}b",
    "A\u{FF06}B",
    "a\u{FF08}b\u{FF09}",
    "a\u{5F4}",
    // Start with punctuation or a combining mark
    ".a",
    "-a",
    "\u{301}a",
    "'a",
    // Double spaces
    "a  b",
    // Hebrew "Ltd."
    "\u{5D1}\u{5E2}\u{5F4}\u{5DE}",
    // Hebrew letter with geresh at the end
    "\u{5E6}\u{5F3}",
    // Tibetan "Tibetan script", with tsheg between syllables
    "\u{F56}\u{F7C}\u{F51}\u{F0B}\u{F61}\u{F72}\u{F42}",
    // Unusual, but allowed
    "0",
    "123",
    // Decomposed (NFD) form
    "e\u{301}",
    // Many combining marks
    "Z\u{334}\u{321}\u{322}a\u{337}l\u{336}g\u{338}o",
    // Mixed scripts
    "\u{410}cme",
    "abc\u{5d1}\u{5e2}\u{5f4}\u{5de}",
];

#[test]
fn project_publisher_parse() {
    for &valid in VALID_PUBLISHERS_NAMES {
        let publisher = ProjectPublisher::parse(valid.to_owned()).unwrap();
        assert_eq!(publisher.as_str(), valid);
        assert_eq!(publisher.into_string(), valid);
    }
    for &(invalid, msg) in INVALID_PUBLISHERS_NAMES {
        assert_eq!(
            ProjectPublisher::parse(invalid.to_owned()).map_err(|(s, e)| (s, e.to_string())),
            Err((invalid.to_owned(), format!("publisher {msg}"))),
            "input: {invalid:?}"
        );
    }
}

#[test]
fn project_name_parse() {
    for &valid in VALID_PUBLISHERS_NAMES {
        let name = ProjectName::parse(valid.to_owned()).unwrap();
        assert_eq!(name.as_str(), valid);
        assert_eq!(name.into_string(), valid);
    }
    for &(invalid, msg) in INVALID_PUBLISHERS_NAMES {
        assert_eq!(
            ProjectName::parse(invalid.to_owned()).map_err(|(s, e)| (s, e.to_string())),
            Err((invalid.to_owned(), format!("name {msg}"))),
            "input: {invalid:?}"
        );
    }
}

#[test]
fn project_field_max_len() {
    let max = "a".repeat(PROJECT_FIELD_MAX_LEN);
    ProjectPublisher::parse(max.clone()).unwrap();
    ProjectName::parse(max).unwrap();
    // Limit is in bytes, not chars (U+0105 is 2 bytes)
    for too_long in [
        "a".repeat(PROJECT_FIELD_MAX_LEN + 1),
        "\u{105}".repeat(PROJECT_FIELD_MAX_LEN / 2 + 1),
    ] {
        let len = too_long.len();
        assert_eq!(
            ProjectName::parse(too_long.clone()).map_err(|(s, e)| (s, e.to_string())),
            Err((
                too_long,
                format!(
                    "name cannot be longer than {PROJECT_FIELD_MAX_LEN} bytes, but is {len} bytes long"
                )
            ))
        );
    }
}

mod index_usage {
    use crate::{
        model::{
            InterchangeProjectUsage, InterchangeProjectUsageRaw, InterchangeProjectValidationError,
        },
        project::utils::Identifier,
    };

    fn parse(json: &str) -> Result<InterchangeProjectUsageRaw, serde_json::Error> {
        serde_json::from_str(json)
    }

    fn index(publisher: &str, name: &str, constraint: &str) -> InterchangeProjectUsageRaw {
        InterchangeProjectUsageRaw::Index {
            publisher: publisher.to_owned(),
            name: name.to_owned(),
            version_constraint: constraint.to_owned(),
        }
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
    fn typed_kinds_reject_extra_keys() {
        for kind in [
            r#""dir":"lib","publisher":"acme","name":"lib""#,
            r#""kparPath":"lib.kpar","publisher":"acme","name":"lib""#,
            r#""publisher":"acme","name":"lib","versionConstraint":"^1""#,
        ] {
            assert!(
                parse(&format!("{{{kind}}}")).is_ok(),
                "{kind} did not parse"
            );
            for key in ["versionConstraint", "dir", "git", "anything"] {
                if !kind.contains(&format!(r#""{key}""#)) {
                    let json = format!(r#"{{{kind},"{key}":"x"}}"#);
                    assert!(parse(&json).is_err(), "{json} parsed");
                }
            }
        }
    }

    #[test]
    fn a_resource_usage_ignores_extra_keys() {
        let usage = parse(
            r#"{"resource":"pkg:sysand/acme/lib","publisher":"acme","name":"lib","x-note":"x"}"#,
        )
        .unwrap();
        assert_eq!(
            usage,
            InterchangeProjectUsageRaw::Resource {
                resource: "pkg:sysand/acme/lib".to_owned(),
                version_constraint: None,
            }
        );
    }

    #[test]
    fn validates() {
        let valid = index("Acme Labs", "My.Lib", "^1.2").validate().unwrap();
        assert_eq!(
            valid,
            InterchangeProjectUsage::Index {
                publisher: crate::model::IndexPublisher::parse("Acme Labs".to_owned()).unwrap(),
                name: crate::model::IndexName::parse("My.Lib".to_owned()).unwrap(),
                version_constraint: semver::VersionReq::parse("^1.2").unwrap(),
            }
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

#[test]
fn index_publisher_and_name_follow_the_unnormalized_purl_rules() {
    use crate::model::{IndexFieldError, IndexName, IndexPublisher};

    for publisher in ["acme", "Acme Labs", "acme-labs", "ACME 2"] {
        let parsed = IndexPublisher::parse(publisher.to_owned()).unwrap();
        assert_eq!(parsed.as_str(), publisher);
    }
    assert_eq!(
        IndexPublisher::parse("Acme Labs".to_owned())
            .unwrap()
            .normalized(),
        "acme-labs"
    );
    // `Foo & Bar` and `Ünï Labs` are valid project publishers, but not
    // valid in an index
    for publisher in [
        "ab",
        "Foo & Bar",
        "acme.labs",
        "acme  labs",
        "-acme",
        "Ünï Labs",
    ] {
        assert_eq!(
            IndexPublisher::parse(publisher.to_owned()),
            Err((publisher.to_owned(), IndexFieldError::Publisher)),
            "{publisher}"
        );
    }
    ProjectPublisher::parse("Foo & Bar".to_owned()).unwrap();

    for name in ["lib", "My.Lib", "my-lib v2"] {
        let parsed = IndexName::parse(name.to_owned()).unwrap();
        assert_eq!(parsed.as_str(), name);
    }
    assert_eq!(
        IndexName::parse("My.Lib".to_owned()).unwrap().normalized(),
        "my.lib"
    );
    for name in ["ab", "my_lib", "my..lib", "my/lib"] {
        assert_eq!(
            IndexName::parse(name.to_owned()),
            Err((name.to_owned(), IndexFieldError::Name)),
            "{name}"
        );
    }

    // Serialized as the plain string, and validated when deserialized
    let publisher = IndexPublisher::parse("Acme Labs".to_owned()).unwrap();
    assert_eq!(serde_json::to_string(&publisher).unwrap(), r#""Acme Labs""#);
    assert_eq!(
        serde_json::from_str::<IndexPublisher>(r#""Acme Labs""#).unwrap(),
        publisher
    );
    serde_json::from_str::<IndexPublisher>(r#""Foo & Bar""#).unwrap_err();
}
