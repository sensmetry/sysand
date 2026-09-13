// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;

use fluent_uri::Iri;
use indexmap::IndexMap;

use crate::{
    model::{
        InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectValidationError, KERML_SPEC_PREFIX, KnownMetamodel, Metamodel,
        MetamodelFamily, MetamodelParseError, MetamodelRelease, PROJECT_FIELD_MAX_LEN, ProjectName,
        ProjectPublisher, SYSML_SPEC_PREFIX, SpecKind,
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
            Err(
                InterchangeProjectValidationError::InvalidIndexUsagePublisher {
                    publisher: _,
                    name: _
                }
            )
        ));
        assert!(matches!(
            index("Acme", "my..lib", "^1").validate(),
            Err(InterchangeProjectValidationError::InvalidIndexUsageName {
                publisher: _,
                name: _
            })
        ));
        assert!(matches!(
            index("Acme", "My Lib", "not a constraint").validate(),
            Err(
                InterchangeProjectValidationError::InvalidIndexUsageVersionConstraint {
                    publisher: _,
                    name: _,
                    constraint: _,
                    source: _
                }
            )
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

#[test]
fn index_publisher_and_name_normalize() {
    use crate::model::{IndexName, IndexPublisher};

    let publisher = |s: &str| IndexPublisher::parse(s.to_owned()).unwrap();
    let name = |s: &str| IndexName::parse(s.to_owned()).unwrap();

    assert_eq!(publisher("ACME LABS").normalized(), "acme-labs");
    assert_eq!(name("My.Project Alpha").normalized(), "my.project-alpha");

    for (s, normalized) in [
        ("acme-labs", true),
        ("Acme Labs", false),
        ("acme labs", false),
        ("ACME", false),
    ] {
        assert_eq!(publisher(s).is_normalized(), normalized, "{s:?}");
        assert_eq!(publisher(s).normalized() == s, normalized, "{s:?}");
    }
    assert!(name("my.lib").is_normalized());
    assert!(!name("My Lib").is_normalized());
}

/// A project publisher or name keeps its spelling, and stores its normalized
/// form
#[test]
fn project_publisher_and_name_store_their_normalized_form() {
    let publisher = ProjectPublisher::parse("Foo & Bar".to_owned()).unwrap();
    assert_eq!(publisher.as_str(), "Foo & Bar");
    assert_eq!(publisher.to_string(), "Foo & Bar");
    assert_eq!(publisher.normalized(), "foo-bar");
    assert_eq!(publisher.clone().into_normalized(), "foo-bar");
    assert_eq!(publisher.into_string(), "Foo & Bar");

    let name = ProjectName::parse("Ａｃｍｅ Lib".to_owned()).unwrap();
    assert_eq!(name.as_str(), "Ａｃｍｅ Lib");
    assert_eq!(name.normalized(), "acme-lib");

    // Serialized as spelled; the normalized form is recomputed when read
    let json = serde_json::to_string(&name).unwrap();
    assert_eq!(json, r#""Ａｃｍｅ Lib""#);
    assert_eq!(serde_json::from_str::<ProjectName>(&json).unwrap(), name);
}

/// Normalization folds every run of separators into a single `-` (keeping a
/// lone `.`), and drops them at either end
#[test]
fn project_fields_normalize_separators() {
    use super::normalize;

    for (spelling, normalized) in [
        ("Acme Labs", "acme-labs"),
        ("My.Lib v2", "my.lib-v2"),
        ("ACME Inc.", "acme-inc"),
        ("AT&T", "at-t"),
        ("C++ Tools", "c-tools"),
        ("Foo (EU)", "foo-eu"),
        ("O'Reilly", "o-reilly"),
        ("Foo, Inc.", "foo-inc"),
        ("my_lib", "my-lib"),
        ("a. b", "a-b"),
        ("a..b", "a-b"),
        ("v1.0 (beta)", "v1.0-beta"),
        // Fullwidth punctuation, which NFKC makes ASCII, separates words too
        ("Acme／Labs", "acme-labs"),
        ("Acme：Labs", "acme-labs"),
        // Fullwidth letters are folded to their ASCII lowercase
        ("Ａｃｍｅ", "acme"),
        // Non-ASCII words are kept, case folded
        ("Ąžuolas Ūkis", "ąžuolas-ūkis"),
    ] {
        assert_eq!(normalize(spelling), normalized, "{spelling:?}");
    }
}

/// An index spelling normalizes as a project spelling the same way it does
/// as an index spelling
#[test]
fn project_normalization_agrees_with_index_normalization() {
    use super::{normalize, normalize_index_field};

    for spelling in [
        "acme",
        "Acme Labs",
        "ACME-LABS-42",
        "My.Project Alpha",
        "my.lib-v2",
        "1.2",
        "4cme",
    ] {
        crate::model::IndexName::parse(spelling.to_owned()).unwrap();
        assert_eq!(
            normalize(spelling),
            normalize_index_field(spelling),
            "{spelling:?}"
        );
    }
}

/// A publisher or name must be valid once normalized too, e.g. not get too
/// long, as NFKC can expand some characters
#[test]
fn project_publisher_and_name_must_be_valid_once_normalized() {
    // U+0F77 is an identifier character that NFKC triples in length
    let field = format!("a{}", "\u{0F77}".repeat(99));
    assert!(field.len() <= 300);
    for err in [
        ProjectPublisher::parse(field.clone()).unwrap_err(),
        ProjectName::parse(field.clone()).unwrap_err(),
    ] {
        assert_eq!(err.0, field);
        let message = err.1.to_string();
        assert!(
            message.contains("normalizes to `a") && message.contains("cannot be longer than 300"),
            "{message}"
        );
    }
}

/// A typed usage's spelling normalizes as an index spells it when it can,
/// and otherwise as a project does
#[test]
fn typed_spellings_normalize_as_index_or_project() {
    use super::{normalize_typed_name, normalize_typed_publisher};

    assert_eq!(normalize_typed_publisher("Acme Labs"), "acme-labs");
    assert_eq!(normalize_typed_name("My.Lib v2"), "my.lib-v2");
    // Not valid in an index: normalized as a project
    assert_eq!(normalize_typed_publisher("Foo & Bar"), "foo-bar");
    assert_eq!(normalize_typed_name("My_Lib"), "my-lib");
    // Not even a valid project field: kept as it is
    assert_eq!(normalize_typed_publisher("a/b"), "a/b");
}

/// A validated directory or KPAR usage holds a project publisher and name,
/// so an invalid one is refused
#[test]
fn directory_and_kpar_usages_validate_their_spelling() {
    use crate::model::{InterchangeProjectUsageRaw, InterchangeProjectValidationError};

    let dir = |publisher: &str, name: &str| InterchangeProjectUsageRaw::Directory {
        dir: "../dep".to_owned(),
        publisher: publisher.to_owned(),
        name: name.to_owned(),
    };
    let kpar = |publisher: &str, name: &str| InterchangeProjectUsageRaw::KparPath {
        kpar_path: "dep.kpar".to_owned(),
        publisher: publisher.to_owned(),
        name: name.to_owned(),
    };

    // Valid for a project, though not for an index
    let crate::model::InterchangeProjectUsage::Directory {
        publisher,
        name,
        dir: _,
    } = dir("Foo & Bar", "My_Lib").validate().unwrap()
    else {
        panic!("a directory usage validates to a directory usage");
    };
    assert_eq!((publisher.as_str(), name.as_str()), ("Foo & Bar", "My_Lib"));

    for usage in [dir("a/b", "lib"), kpar("a/b", "lib")] {
        assert!(
            matches!(
                usage.validate(),
                Err(InterchangeProjectValidationError::InvalidUsagePublisher {
                    publisher: _,
                    name: _,
                    source: _
                })
            ),
            "{usage:?}"
        );
    }
    for usage in [dir("acme", ""), kpar("acme", "")] {
        assert!(
            matches!(
                usage.validate(),
                Err(InterchangeProjectValidationError::InvalidUsageName {
                    publisher: _,
                    name: _,
                    source: _
                })
            ),
            "{usage:?}"
        );
    }
}

#[test]
fn urn_sysand_resource_usage_is_rejected() {
    use crate::model::{InterchangeProjectUsageRaw, InterchangeProjectValidationError};

    for resource in [
        "urn:sysand:acme-inc/foo",
        "URN:Sysand:acme-inc/foo",
        "urn:sysand:",
    ] {
        let usage = InterchangeProjectUsageRaw::Resource {
            resource: resource.to_owned(),
            version_constraint: None,
        };
        let err = usage.validate().unwrap_err();
        assert!(
            matches!(&err, InterchangeProjectValidationError::UrnSysandUsage(r) if r == resource),
            "{err:?}"
        );
    }

    // Only the `urn:sysand` namespace is internal
    for resource in [
        "urn:sysandx:acme-inc/foo",
        "urn:kpar:sysand",
        "urn:x:urn:sysand:a/b",
    ] {
        InterchangeProjectUsageRaw::Resource {
            resource: resource.to_owned(),
            version_constraint: None,
        }
        .validate()
        .unwrap();
    }
}

#[test]
fn metamodel_family_prefixes_and_names() {
    assert_eq!(MetamodelFamily::Omg.prefix(), "https://www.omg.org/spec/");
    assert!(SYSML_SPEC_PREFIX.starts_with(MetamodelFamily::Omg.prefix()));
    assert!(KERML_SPEC_PREFIX.starts_with(MetamodelFamily::Omg.prefix()));
    assert_eq!(
        MetamodelFamily::Syside.prefix(),
        "https://www.sysand.com/spec/"
    );
    assert_eq!(MetamodelFamily::Omg.to_string(), "OMG");
    assert_eq!(MetamodelFamily::Syside.to_string(), "Syside");
}

const OMG_SYSML: &str = "https://www.omg.org/spec/SysML/20250201";
const OMG_KERML: &str = "https://www.omg.org/spec/KerML/20250201";
const SYSIDE_SYSML: &str = "https://www.sysand.com/spec/SysML/20250201";
const SYSIDE_KERML: &str = "https://www.sysand.com/spec/KerML/20250201";

/// model-toml.md "Metamodel": a recognized value is
/// `<family prefix><spec-kind>/<date>`, in either of the two families.
#[test]
fn metamodel_parses_both_families() {
    for (value, family, spec) in [
        (OMG_SYSML, MetamodelFamily::Omg, SpecKind::SysMl),
        (OMG_KERML, MetamodelFamily::Omg, SpecKind::KerMl),
        (SYSIDE_SYSML, MetamodelFamily::Syside, SpecKind::SysMl),
        (SYSIDE_KERML, MetamodelFamily::Syside, SpecKind::KerMl),
    ] {
        assert_eq!(
            metamodel(value),
            Metamodel::Known(KnownMetamodel {
                family,
                spec,
                release: MetamodelRelease::Release20250201
            })
        );
    }
}

/// The value is rebuilt from the parts when a standard-library usage IRI
/// is composed from the target's metamodel, so it has to be the value that
/// was read.
#[test]
fn metamodel_round_trips_through_display() {
    for value in [
        OMG_SYSML,
        OMG_KERML,
        SYSIDE_SYSML,
        SYSIDE_KERML,
        "https://example.com/x",
    ] {
        assert_eq!(metamodel(value).to_string(), value);
        assert_eq!(String::from(metamodel(value)), value);
    }
}

/// Values without the prefix of a recognized metamodel are some other
/// metamodel; a well-formed release that does not exist is still known.
#[test]
fn metamodel_parses_other_values() {
    for value in [
        "https://example.com/spec/SysML/20250201",
        "http://www.omg.org/spec/SysML/20250201",
        "https://www.omg.org/spec/UML/20250201",
        "https://www.omg.org/spec/SysML",
    ] {
        assert_eq!(
            metamodel(value),
            Metamodel::Other(Iri::parse(value.to_owned()).unwrap()),
            "{value} should not be recognized"
        );
    }

    assert_eq!(
        metamodel("https://www.omg.org/spec/SysML/20991299")
            .known()
            .map(|known| known.release.as_str()),
        Some("20991299")
    );
}

/// A recognized prefix followed by anything other than a release date is
/// an error, reported with the date exactly as written.
#[test]
fn metamodel_rejects_a_malformed_release() {
    for (value, release) in [
        ("https://www.omg.org/spec/SysML/", ""),
        ("https://www.omg.org/spec/SysML/2025020", "2025020"),
        ("https://www.omg.org/spec/SysML/2025020a", "2025020a"),
        ("https://www.omg.org/spec/KerML/20251301", "20251301"),
        ("https://www.omg.org/spec/KerML/20250001", "20250001"),
        ("https://www.sysand.com/spec/SysML/19250201", "19250201"),
        ("https://www.omg.org/spec/SysML/20250201/", "20250201/"),
        (
            "https://www.omg.org/spec/SysML/20250201/extra",
            "20250201/extra",
        ),
    ] {
        assert_eq!(
            Metamodel::parse(Iri::parse(value.to_owned()).unwrap()),
            Err(MetamodelParseError {
                metamodel: value.to_owned(),
                release: release.to_owned()
            }),
            "{value} should have an invalid release"
        );
    }
}

/// model-toml.md "Metamodel": "A project without a `metamodel` is treated
/// as an OMG-metamodel project"; an unrecognized value is validated
/// against the OMG schema (rule (a)).
#[test]
fn metamodel_schema_family_of_a_project() {
    assert_eq!(Metamodel::schema_family(None), MetamodelFamily::Omg);
    assert_eq!(
        Metamodel::schema_family(Some(&metamodel(OMG_SYSML))),
        MetamodelFamily::Omg
    );
    assert_eq!(
        Metamodel::schema_family(Some(&metamodel(SYSIDE_KERML))),
        MetamodelFamily::Syside
    );
    let other = metamodel("https://example.com/x");
    assert_eq!(Metamodel::schema_family(Some(&other)), MetamodelFamily::Omg);
    assert_eq!(other.known(), None);
}

/// Validation parses the metamodel; only a malformed release of a
/// recognized metamodel is an error.
#[test]
fn validate_parses_the_metamodel() {
    let raw = |value: &str| InterchangeProjectMetadataRaw {
        index: IndexMap::new(),
        created: "2026-01-01T00:00:00Z".to_owned(),
        metamodel: Some(value.to_owned()),
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    };
    for value in [
        "https://www.omg.org/spec/SysML/20250601",
        SYSIDE_KERML,
        "https://example.com/x",
    ] {
        let validated = raw(value).validate().expect("a valid metadata document");
        assert_eq!(validated.metamodel, Some(metamodel(value)));
    }

    assert_matches!(
        raw("https://www.omg.org/spec/SysML/2025").validate(),
        Err(InterchangeProjectValidationError::MalformedMetamodel(_))
    );
}

#[test]
fn metamodel_release_shape() {
    assert_eq!(
        MetamodelRelease::parse("20250201"),
        Some(MetamodelRelease::Release20250201)
    );
    for date in ["20991299", "20000100"] {
        let release = MetamodelRelease::parse(date).expect("a well-formed release");
        assert_matches!(release, MetamodelRelease::Other(_));
        assert_eq!(release.as_str(), date);
    }
    for date in [
        "",
        "2025020",
        "202502011",
        "19250201",
        "20251301",
        "2025-201",
        "+0250201",
    ] {
        assert!(MetamodelRelease::parse(date).is_none(), "{date}");
    }
}

fn metamodel(value: &str) -> Metamodel {
    Metamodel::parse(Iri::parse(value.to_owned()).unwrap()).expect("a well-formed metamodel")
}
