// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use serde::{Deserialize, Serialize};

use crate::manifest::serde_support::{IriValue, LicenseExpr, PathValue, Reserved};

#[derive(Debug, Deserialize, Serialize)]
struct Values {
    website: IriValue,
    path: PathValue,
    license: LicenseExpr,
}

#[derive(Debug, Deserialize)]
struct WithReserved {
    #[expect(dead_code)]
    root: Reserved,
}

#[test]
fn iri_value_round_trips_through_a_string() {
    let values: Values = toml_edit::de::from_str(
        "website = \"https://example.com/a\"\npath = \"./a/b.sysml\"\nlicense = \"MIT OR Apache-2.0\"\n",
    )
    .expect("parses");

    assert_eq!(values.website.0.as_str(), "https://example.com/a");
    assert_eq!(
        toml_edit::ser::to_string(&values).expect("serializes"),
        "website = \"https://example.com/a\"\npath = \"./a/b.sysml\"\nlicense = \"MIT OR Apache-2.0\"\n"
    );
}

/// `fluent_uri` implements `Deserialize` itself, but reports a failure as
/// "failed to parse `<debug of the error>` as IRI: `<nothing>`", with the
/// error and the input the wrong way round. `IriValue` exists to report
/// the `ParseError` itself.
#[test]
fn iri_value_reports_the_parse_error() {
    #[derive(Debug, Deserialize)]
    struct OneIri {
        #[expect(dead_code)]
        website: IriValue,
    }

    let error = toml_edit::de::from_str::<OneIri>("website = \"not an iri\"\n")
        .expect_err("a space is not allowed in an IRI");
    let expected = fluent_uri::Iri::parse("not an iri".to_owned())
        .expect_err("a space is not allowed in an IRI")
        .0
        .to_string();

    assert!(
        error.to_string().contains(&expected),
        "unexpected error: {error}"
    );
}

/// Normalizing a path is the job of the collection that owns it
/// (`Sources`), not of the wrapper that reads it.
#[test]
fn path_value_keeps_the_written_spelling() {
    #[derive(Debug, Deserialize, Serialize)]
    struct OnePath {
        path: PathValue,
    }

    let one: OnePath = toml_edit::de::from_str("path = \"./a/../b.sysml\"\n").expect("parses");
    assert_eq!(one.path.0.as_str(), "./a/../b.sysml");
    assert_eq!(
        toml_edit::ser::to_string(&one).expect("serializes"),
        "path = \"./a/../b.sysml\"\n"
    );
}

#[test]
fn license_expr_parses_an_spdx_expression() {
    #[derive(Debug, Deserialize, Serialize)]
    struct OneLicense {
        license: LicenseExpr,
    }

    let one: OneLicense =
        toml_edit::de::from_str("license = \"MIT OR Apache-2.0\"\n").expect("parses");
    assert_eq!(one.license.0.to_string(), "MIT OR Apache-2.0");

    let error = toml_edit::de::from_str::<OneLicense>("license = \"NOT-A-LICENSE\"\n")
        .expect_err("not an SPDX expression");
    assert!(
        error.to_string().contains("unknown term"),
        "unexpected error: {error}"
    );
}

/// `root` is reserved for the Syside metamodel: the reader keeps no value
/// for it, only the fact that it was written, so that it can warn.
#[test]
fn reserved_accepts_any_value() {
    for value in ["root = { a = 1 }", "root = \"anything\"", "root = 3"] {
        toml_edit::de::from_str::<WithReserved>(&format!("{value}\n"))
            .unwrap_or_else(|e| panic!("`{value}` should deserialize: {e}"));
    }
}
