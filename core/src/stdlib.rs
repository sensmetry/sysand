// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::collections::HashMap;

use crate::{
    project::{memory::InMemoryProject, utils::Identifier},
    utils::ProvidedProjects,
};

const QUANTITIES_AND_UNITS_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/quantities-and-units-library.project.json");
const QUANTITIES_AND_UNITS_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/quantities-and-units-library.meta.json");
const FUNCTION_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/function-library.project.json");
const FUNCTION_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/function-library.meta.json");
const SYSTEMS_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/systems-library.project.json");
const SYSTEMS_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/systems-library.meta.json");
const CAUSE_AND_EFFECT_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/cause-and-effect-library.project.json");
const CAUSE_AND_EFFECT_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/cause-and-effect-library.meta.json");
const REQUIREMENT_DERIVATION_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/requirement-derivation-library.project.json");
const REQUIREMENT_DERIVATION_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/requirement-derivation-library.meta.json");
const METADATA_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/metadata-library.project.json");
const METADATA_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/metadata-library.meta.json");
const GEOMETRY_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/geometry-library.project.json");
const GEOMETRY_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/geometry-library.meta.json");
const ANALYSIS_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/analysis-library.project.json");
const ANALYSIS_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/analysis-library.meta.json");
const DATA_TYPE_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/data-type-library.project.json");
const DATA_TYPE_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/data-type-library.meta.json");
const SEMANTIC_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/semantic-library.project.json");
const SEMANTIC_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/semantic-library.meta.json");

/// The ten releases this Sysand embeds: IRI and `.project.json`.
const STD_LIB_ASSETS_20250201: [(&str, &str); 10] = [
    (
        "https://www.omg.org/spec/SysML/20250201/Quantities-and-Units-Domain-Library.kpar",
        QUANTITIES_AND_UNITS_LIBRARY_INFO_20250201,
    ),
    (
        "https://www.omg.org/spec/KerML/20250201/Function-Library.kpar",
        FUNCTION_LIBRARY_INFO_20250201,
    ),
    (
        "https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar",
        SYSTEMS_LIBRARY_INFO_20250201,
    ),
    (
        "https://www.omg.org/spec/SysML/20250201/Cause-and-Effect-Domain-Library.kpar",
        CAUSE_AND_EFFECT_LIBRARY_INFO_20250201,
    ),
    (
        "https://www.omg.org/spec/SysML/20250201/Requirement-Derivation-Domain-Library.kpar",
        REQUIREMENT_DERIVATION_LIBRARY_INFO_20250201,
    ),
    (
        "https://www.omg.org/spec/SysML/20250201/Metadata-Domain-Library.kpar",
        METADATA_LIBRARY_INFO_20250201,
    ),
    (
        "https://www.omg.org/spec/SysML/20250201/Geometry-Domain-Library.kpar",
        GEOMETRY_LIBRARY_INFO_20250201,
    ),
    (
        "https://www.omg.org/spec/SysML/20250201/Analysis-Domain-Library.kpar",
        ANALYSIS_LIBRARY_INFO_20250201,
    ),
    (
        "https://www.omg.org/spec/KerML/20250201/Data-Type-Library.kpar",
        DATA_TYPE_LIBRARY_INFO_20250201,
    ),
    (
        "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar",
        SEMANTIC_LIBRARY_INFO_20250201,
    ),
];

// TODO: These should not be hard-coded, this is just a stop-gap solution
// even if we keep some of these hard-coded it might be neater if we can
// embed the .project.json and .meta.json files separately
// TODO: use std::cell::Lazy (or similar), since this does not need
// to be recreated on each call
pub fn known_std_libs() -> ProvidedProjects {
    fn entries(
        xs: impl IntoIterator<Item = (&'static str, &'static str, &'static str)>,
    ) -> ProvidedProjects {
        let mut result = HashMap::default();

        for (iri, info, meta) in xs {
            let projects = result
                .entry(Identifier::from_iri_unchecked_str(iri))
                .or_insert_with(Vec::new);
            projects.push(InMemoryProject::from_info_meta(
                serde_json::from_str(info).unwrap(),
                serde_json::from_str(meta).unwrap(),
            ));
        }

        result
    }

    entries([
        (
            "https://www.omg.org/spec/SysML/20250201/Quantities-and-Units-Domain-Library.kpar",
            QUANTITIES_AND_UNITS_LIBRARY_INFO_20250201,
            QUANTITIES_AND_UNITS_LIBRARY_META_20250201,
        ),
        (
            "https://www.omg.org/spec/KerML/20250201/Function-Library.kpar",
            FUNCTION_LIBRARY_INFO_20250201,
            FUNCTION_LIBRARY_META_20250201,
        ),
        (
            "https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar",
            SYSTEMS_LIBRARY_INFO_20250201,
            SYSTEMS_LIBRARY_META_20250201,
        ),
        (
            "https://www.omg.org/spec/SysML/20250201/Cause-and-Effect-Domain-Library.kpar",
            CAUSE_AND_EFFECT_LIBRARY_INFO_20250201,
            CAUSE_AND_EFFECT_LIBRARY_META_20250201,
        ),
        (
            "https://www.omg.org/spec/SysML/20250201/Requirement-Derivation-Domain-Library.kpar",
            REQUIREMENT_DERIVATION_LIBRARY_INFO_20250201,
            REQUIREMENT_DERIVATION_LIBRARY_META_20250201,
        ),
        (
            "https://www.omg.org/spec/SysML/20250201/Metadata-Domain-Library.kpar",
            METADATA_LIBRARY_INFO_20250201,
            METADATA_LIBRARY_META_20250201,
        ),
        (
            "https://www.omg.org/spec/SysML/20250201/Geometry-Domain-Library.kpar",
            GEOMETRY_LIBRARY_INFO_20250201,
            GEOMETRY_LIBRARY_META_20250201,
        ),
        (
            "https://www.omg.org/spec/SysML/20250201/Analysis-Domain-Library.kpar",
            ANALYSIS_LIBRARY_INFO_20250201,
            ANALYSIS_LIBRARY_META_20250201,
        ),
        (
            "https://www.omg.org/spec/KerML/20250201/Data-Type-Library.kpar",
            DATA_TYPE_LIBRARY_INFO_20250201,
            DATA_TYPE_LIBRARY_META_20250201,
        ),
        (
            "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar",
            SEMANTIC_LIBRARY_INFO_20250201,
            SEMANTIC_LIBRARY_META_20250201,
        ),
    ])
}

/// The name the standard library published at `iri` declares, or `None`
/// if this Sysand embeds no release at that IRI.
///
/// model-toml.md "Standard library": a usage's `name` "must be the name
/// the KPAR declares". That name is not the IRI's stem -- `SysML
/// Quantities and Units Library` is published at
/// `.../Quantities-and-Units-Domain-Library.kpar`, and seven of the ten
/// differ by more than punctuation -- so anything writing such a usage
/// asks here rather than guessing, and two spellings of one library
/// cannot become two identities.
///
/// Built from the ten `.project.json` documents alone: `known_std_libs`
/// would also parse ten `.meta.json` documents and build ten
/// `InMemoryProject`s, which a caller that wants a name does not need.
pub fn std_lib_name(iri: &str) -> Option<&'static str> {
    static NAMES: std::sync::LazyLock<HashMap<&'static str, String>> =
        std::sync::LazyLock::new(|| {
            STD_LIB_ASSETS_20250201
                .iter()
                .filter_map(|(iri, info)| {
                    let parsed: crate::model::InterchangeProjectInfoRaw =
                        serde_json::from_str(info).ok()?;
                    Some((*iri, parsed.name))
                })
                .collect()
        });
    // The map owns its `String`s for the process's lifetime, so handing
    // out a `&'static str` is sound and saves every caller a clone.
    NAMES.get(iri).map(String::as_str)
}
