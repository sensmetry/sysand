// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Local newtypes for values a manifest carries as a plain string.
//!
//! `typed_path::Utf8UnixPathBuf` and `spdx::Expression` have no `serde`
//! support of their own, and Rust's orphan rules block implementing a
//! foreign trait for a foreign type, so the manifest structs use these
//! wrappers wherever such a value appears. `fluent_uri::Iri` does
//! implement `serde` (the `serde` feature is on in this crate), but its
//! `Deserialize` renders a failure with the error and the input the wrong
//! way round, so [`IriValue`] is used for the message rather than for the
//! missing impl.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use spdx::Expression;
use typed_path::Utf8UnixPathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IriValue(pub fluent_uri::Iri<String>);

impl Serialize for IriValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for IriValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        fluent_uri::Iri::parse(s)
            .map(IriValue)
            .map_err(|(e, _)| serde::de::Error::custom(e))
    }
}

/// A manifest path, as written. Normalization and the rules a path has to
/// satisfy belong to the collection that owns it, such as
/// [`Sources`](crate::manifest::common::Sources), not here.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PathValue(pub Utf8UnixPathBuf);

impl Serialize for PathValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PathValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self(Utf8UnixPathBuf::from(s)))
    }
}

/// model-toml.md `license`: an SPDX *expression*, with identifiers
/// validated against the license list the `spdx` crate carries.
#[derive(Clone, Debug)]
pub struct LicenseExpr(pub Expression);

impl Serialize for LicenseExpr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LicenseExpr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Expression::parse(&s)
            .map(LicenseExpr)
            .map_err(serde::de::Error::custom)
    }
}

/// A key whose *presence* is all the reader keeps. Used for `root`, which
/// model-toml.md reserves for the Syside metamodel: no value is kept for
/// it, only the fact that it was written, so that it can warn
/// ([`ReadWarning::ForeignSchemaKey`](crate::manifest::diagnostics::ReadWarning::ForeignSchemaKey)).
/// Deserialize-only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reserved;

impl<'de> Deserialize<'de> for Reserved {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(deserializer)?;
        Ok(Self)
    }
}

#[cfg(test)]
#[path = "./serde_support_tests.rs"]
mod tests;
