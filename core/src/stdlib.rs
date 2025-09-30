use std::collections::HashMap;

use crate::project::memory::InMemoryProject;

const QUANTITIES_AND_UNITS_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/quantities-and-units-library.project.json");
const QUANTITIES_AND_UNITS_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/quantities-and-units-library.meta.json");
const FUNCTION_LIBRARY_INFO_20250201: &str = include_str!("stdlib_assets/20250201/function-library.project.json");
const FUNCTION_LIBRARY_META_20250201: &str = include_str!("stdlib_assets/20250201/function-library.meta.json");
const SYSTEMS_LIBRARY_INFO_20250201: &str = include_str!("stdlib_assets/20250201/systems-library.project.json");
const SYSTEMS_LIBRARY_META_20250201: &str = include_str!("stdlib_assets/20250201/systems-library.meta.json");
const CAUSE_AND_EFFECT_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/cause-and-effect-library.project.json");
const CAUSE_AND_EFFECT_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/cause-and-effect-library.meta.json");
const REQUIREMENT_DERIVATION_LIBRARY_INFO_20250201: &str =
    include_str!("stdlib_assets/20250201/requirement-derivation-library.project.json");
const REQUIREMENT_DERIVATION_LIBRARY_META_20250201: &str =
    include_str!("stdlib_assets/20250201/requirement-derivation-library.meta.json");
const METADATA_LIBRARY_INFO_20250201: &str = include_str!("stdlib_assets/20250201/metadata-library.project.json");
const METADATA_LIBRARY_META_20250201: &str = include_str!("stdlib_assets/20250201/metadata-library.meta.json");
const GEOMETRY_LIBRARY_INFO_20250201: &str = include_str!("stdlib_assets/20250201/geometry-library.project.json");
const GEOMETRY_LIBRARY_META_20250201: &str = include_str!("stdlib_assets/20250201/geometry-library.meta.json");
const ANALYSIS_LIBRARY_INFO_20250201: &str = include_str!("stdlib_assets/20250201/analysis-library.project.json");
const ANALYSIS_LIBRARY_META_20250201: &str = include_str!("stdlib_assets/20250201/analysis-library.meta.json");
const DATA_TYPE_LIBRARY_INFO_20250201: &str = include_str!("stdlib_assets/20250201/data-type-library.project.json");
const DATA_TYPE_LIBRARY_META_20250201: &str = include_str!("stdlib_assets/20250201/data-type-library.meta.json");
const SEMANTIC_LIBRARY_INFO_20250201: &str = include_str!("stdlib_assets/20250201/semantic-library.project.json");
const SEMANTIC_LIBRARY_META_20250201: &str = include_str!("stdlib_assets/20250201/semantic-library.meta.json");

const QUANTITIES_AND_UNITS_LIBRARY_INFO_20230201: &str =
    include_str!("stdlib_assets/20230201/quantities-and-units-library.project.json");
const QUANTITIES_AND_UNITS_LIBRARY_META_20230201: &str =
    include_str!("stdlib_assets/20230201/quantities-and-units-library.meta.json");
const FUNCTION_LIBRARY_INFO_20230201: &str = include_str!("stdlib_assets/20230201/function-library.project.json");
const FUNCTION_LIBRARY_META_20230201: &str = include_str!("stdlib_assets/20230201/function-library.meta.json");
const SYSTEMS_LIBRARY_INFO_20230201: &str = include_str!("stdlib_assets/20230201/systems-library.project.json");
const SYSTEMS_LIBRARY_META_20230201: &str = include_str!("stdlib_assets/20230201/systems-library.meta.json");
const CAUSE_AND_EFFECT_LIBRARY_INFO_20230201: &str =
    include_str!("stdlib_assets/20230201/cause-and-effect-library.project.json");
const CAUSE_AND_EFFECT_LIBRARY_META_20230201: &str =
    include_str!("stdlib_assets/20230201/cause-and-effect-library.meta.json");
const REQUIREMENT_DERIVATION_LIBRARY_INFO_20230201: &str =
    include_str!("stdlib_assets/20230201/requirement-derivation-library.project.json");
const REQUIREMENT_DERIVATION_LIBRARY_META_20230201: &str =
    include_str!("stdlib_assets/20230201/requirement-derivation-library.meta.json");
const METADATA_LIBRARY_INFO_20230201: &str = include_str!("stdlib_assets/20230201/metadata-library.project.json");
const METADATA_LIBRARY_META_20230201: &str = include_str!("stdlib_assets/20230201/metadata-library.meta.json");
const GEOMETRY_LIBRARY_INFO_20230201: &str = include_str!("stdlib_assets/20230201/geometry-library.project.json");
const GEOMETRY_LIBRARY_META_20230201: &str = include_str!("stdlib_assets/20230201/geometry-library.meta.json");
const ANALYSIS_LIBRARY_INFO_20230201: &str = include_str!("stdlib_assets/20230201/analysis-library.project.json");
const ANALYSIS_LIBRARY_META_20230201: &str = include_str!("stdlib_assets/20230201/analysis-library.meta.json");
const DATA_TYPE_LIBRARY_INFO_20230201: &str = include_str!("stdlib_assets/20230201/data-type-library.project.json");
const DATA_TYPE_LIBRARY_META_20230201: &str = include_str!("stdlib_assets/20230201/data-type-library.meta.json");
const SEMANTIC_LIBRARY_INFO_20230201: &str = include_str!("stdlib_assets/20230201/semantic-library.project.json");
const SEMANTIC_LIBRARY_META_20230201: &str = include_str!("stdlib_assets/20230201/semantic-library.meta.json");

// TODO: These should not be hard-coded, this is just a stop-gap solution
// even if we keep some of these hard-coded it might be neater if we can
// embed the .project.json and .meta.json files separately
pub fn known_std_libs() -> std::collections::HashMap<String, Vec<InMemoryProject>> {
    fn entries(
        xs: impl IntoIterator<Item = (&'static str, &'static str, &'static str)>,
    ) -> std::collections::HashMap<String, Vec<InMemoryProject>> {
        let mut result = HashMap::default();

        for (iri, info, meta) in xs {
            let projects = result.entry(iri.to_string()).or_insert_with(Vec::new);
            projects.push(InMemoryProject::from_info_meta(
                serde_json::from_str(info).unwrap(),
                serde_json::from_str(meta).unwrap(),
            ));
        }

        result
    }

    entries([
        (
            "urn:kpar:quantities-and-units-library",
            QUANTITIES_AND_UNITS_LIBRARY_INFO_20250201,
            QUANTITIES_AND_UNITS_LIBRARY_META_20250201,
        ),
        (
            "urn:kpar:function-library",
            FUNCTION_LIBRARY_INFO_20250201,
            FUNCTION_LIBRARY_META_20250201,
        ),
        (
            "urn:kpar:systems-library",
            SYSTEMS_LIBRARY_INFO_20250201,
            SYSTEMS_LIBRARY_META_20250201,
        ),
        (
            "urn:kpar:cause-and-effect-library",
            CAUSE_AND_EFFECT_LIBRARY_INFO_20250201,
            CAUSE_AND_EFFECT_LIBRARY_META_20250201,
        ),
        (
            "urn:kpar:requirement-derivation-library",
            REQUIREMENT_DERIVATION_LIBRARY_INFO_20250201,
            REQUIREMENT_DERIVATION_LIBRARY_META_20250201,
        ),
        (
            "urn:kpar:metadata-library",
            METADATA_LIBRARY_INFO_20250201,
            METADATA_LIBRARY_META_20250201,
        ),
        (
            "urn:kpar:geometry-library",
            GEOMETRY_LIBRARY_INFO_20250201,
            GEOMETRY_LIBRARY_META_20250201,
        ),
        (
            "urn:kpar:analysis-library",
            ANALYSIS_LIBRARY_INFO_20250201,
            ANALYSIS_LIBRARY_META_20250201,
        ),
        (
            "urn:kpar:data-type-library",
            DATA_TYPE_LIBRARY_INFO_20250201,
            DATA_TYPE_LIBRARY_META_20250201,
        ),
        (
            "urn:kpar:semantic-library",
            SEMANTIC_LIBRARY_INFO_20250201,
            SEMANTIC_LIBRARY_META_20250201,
        ),
        //
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
        //
        (
            "https://www.omg.org/spec/SysML/20230201/Quantities-and-Units-Domain-Library.kpar",
            QUANTITIES_AND_UNITS_LIBRARY_INFO_20230201,
            QUANTITIES_AND_UNITS_LIBRARY_META_20230201,
        ),
        (
            "https://www.omg.org/spec/KerML/20230201/Function-Library.kpar",
            FUNCTION_LIBRARY_INFO_20230201,
            FUNCTION_LIBRARY_META_20230201,
        ),
        (
            "https://www.omg.org/spec/SysML/20230201/Systems-Library.kpar",
            SYSTEMS_LIBRARY_INFO_20230201,
            SYSTEMS_LIBRARY_META_20230201,
        ),
        (
            "https://www.omg.org/spec/SysML/20230201/Cause-and-Effect-Domain-Library.kpar",
            CAUSE_AND_EFFECT_LIBRARY_INFO_20230201,
            CAUSE_AND_EFFECT_LIBRARY_META_20230201,
        ),
        (
            "https://www.omg.org/spec/SysML/20230201/Requirement-Derivation-Domain-Library.kpar",
            REQUIREMENT_DERIVATION_LIBRARY_INFO_20230201,
            REQUIREMENT_DERIVATION_LIBRARY_META_20230201,
        ),
        (
            "https://www.omg.org/spec/SysML/20230201/Metadata-Domain-Library.kpar",
            METADATA_LIBRARY_INFO_20230201,
            METADATA_LIBRARY_META_20230201,
        ),
        (
            "https://www.omg.org/spec/SysML/20230201/Geometry-Domain-Library.kpar",
            GEOMETRY_LIBRARY_INFO_20230201,
            GEOMETRY_LIBRARY_META_20230201,
        ),
        (
            "https://www.omg.org/spec/SysML/20230201/Analysis-Domain-Library.kpar",
            ANALYSIS_LIBRARY_INFO_20230201,
            ANALYSIS_LIBRARY_META_20230201,
        ),
        (
            "https://www.omg.org/spec/KerML/20230201/Data-Type-Library.kpar",
            DATA_TYPE_LIBRARY_INFO_20230201,
            DATA_TYPE_LIBRARY_META_20230201,
        ),
        (
            "https://www.omg.org/spec/KerML/20230201/Semantic-Library.kpar",
            SEMANTIC_LIBRARY_INFO_20230201,
            SEMANTIC_LIBRARY_META_20230201,
        ),
    ])
}
