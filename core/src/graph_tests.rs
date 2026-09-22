// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::graph::{DependencyGraph, check_metamodel_pair};
use crate::model::Metamodel;

const OMG_SYSML_2025: &str = "https://www.omg.org/spec/SysML/20250201";
const OMG_KERML_2025: &str = "https://www.omg.org/spec/KerML/20250201";
const OMG_KERML_2024: &str = "https://www.omg.org/spec/KerML/20240201";
const SYSIDE_SYSML_2025: &str = "https://www.sysand.com/spec/SysML/20250201";

/// A graph over the node names given, with an edge for every
/// `(dependent, dependency)` pair. Edges whose endpoints are all nodes,
/// which is what every test but one wants.
fn graph(nodes: &[&str], edges: &[(&str, &str)]) -> DependencyGraph<String> {
    let mut graph = DependencyGraph::new();
    for node in nodes {
        graph.add_node((*node).to_owned());
    }
    for (from, to) in edges {
        assert!(
            graph.add_edge(&(*from).to_owned(), &(*to).to_owned()),
            "{from} -> {to} should be an edge between known nodes"
        );
    }
    graph
}

fn order(graph: &DependencyGraph<String>) -> Vec<String> {
    graph.dependency_order().into_iter().cloned().collect()
}

/// workspaces.md "Build output": members are published in dependency
/// order, a topological sort of their `dir` usages of each other.
#[test]
fn dependency_order_is_dependencies_first() {
    assert_eq!(
        order(&graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")])),
        ["c", "b", "a"]
    );
    assert_eq!(
        order(&graph(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")]
        )),
        ["d", "b", "c", "a"]
    );
    assert_eq!(order(&graph(&["a", "b", "c"], &[])), ["a", "b", "c"]);
}

/// A cycle has no dependency order, so its projects keep the order they
/// were added in, and the rest of the graph is still ordered around it.
#[test]
fn dependency_order_keeps_a_cycle_in_insertion_order() {
    let graph = graph(&["a", "b", "c"], &[("a", "b"), ("b", "a"), ("c", "a")]);
    assert_eq!(order(&graph), ["a", "b", "c"]);
}

/// workspaces.md: "A dependency cycle is a hard error listing the cycle".
#[test]
fn cycles_name_the_component_and_a_path_through_it() {
    let graph = graph(
        &["acme/a", "acme/b", "acme/c"],
        &[
            ("acme/a", "acme/b"),
            ("acme/b", "acme/a"),
            ("acme/c", "acme/a"),
        ],
    );

    let [cycle] = graph.cycles().try_into().expect("one cycle");
    assert_eq!(cycle.component, ["acme/a", "acme/b"]);
    assert_eq!(cycle.path, ["acme/a", "acme/b", "acme/a"]);
    assert_eq!(
        cycle.to_string(),
        "dependency cycle: `acme/a` -> `acme/b` -> `acme/a`; \
         only projects that carry `stdlib` may form a cycle"
    );
}

#[test]
fn a_self_usage_is_a_cycle() {
    let graph = graph(&["a"], &[("a", "a")]);

    let [cycle] = graph.cycles().try_into().expect("one cycle");
    assert_eq!(cycle.component, ["a"]);
    assert_eq!(cycle.path, ["a", "a"]);
    assert_eq!(order(&graph), ["a"]);
}

#[test]
fn cycles_lists_every_component_that_is_one() {
    let graph = graph(
        &["a", "b", "c", "d", "e"],
        &[
            ("a", "b"),
            ("b", "a"),
            ("c", "d"),
            ("d", "c"),
            ("e", "a"),
            ("e", "c"),
        ],
    );

    let cycles: Vec<Vec<String>> = graph
        .cycles()
        .into_iter()
        .map(|cycle| cycle.component)
        .collect();
    assert_eq!(cycles.len(), 2);
    assert!(cycles.contains(&vec!["a".to_owned(), "b".to_owned()]));
    assert!(cycles.contains(&vec!["c".to_owned(), "d".to_owned()]));
}

/// A member's usages usually include projects that are not members; those
/// are not edges of this graph, and must not become nodes of it.
#[test]
fn an_edge_to_an_unknown_node_is_ignored() {
    let mut graph = DependencyGraph::new();
    graph.add_node("a".to_owned());

    assert!(!graph.add_edge(&"a".to_owned(), &"outsider".to_owned()));
    assert!(!graph.add_edge(&"outsider".to_owned(), &"a".to_owned()));
    assert_eq!(graph.nodes(), ["a"]);
    assert_eq!(order(&graph), ["a"]);
}

#[test]
fn graph_deduplicates_nodes_and_edges() {
    let mut graph = DependencyGraph::new();
    let first = graph.add_node("a".to_owned());
    assert_eq!(graph.add_node("a".to_owned()), first);
    graph.add_node("b".to_owned());

    assert!(graph.add_edge(&"a".to_owned(), &"b".to_owned()));
    assert!(graph.add_edge(&"a".to_owned(), &"b".to_owned()));
    assert_eq!(graph.nodes().len(), 2);
    assert_eq!(order(&graph), ["b", "a"]);
}

/// The traversal is iterative, so a deep chain is a matter of memory
/// rather than of stack depth.
#[test]
fn dependency_order_handles_a_long_chain_without_overflowing() {
    const LENGTH: usize = 50_000;

    let mut graph = DependencyGraph::new();
    for node in 0..LENGTH {
        graph.add_node(node);
    }
    for node in 1..LENGTH {
        assert!(graph.add_edge(&(node - 1), &node));
    }

    let order = graph.dependency_order();
    assert_eq!(order.len(), LENGTH);
    assert_eq!(*order[0], LENGTH - 1);
    assert_eq!(*order[LENGTH - 1], 0);
}

// ----------------------------------------------------------------------
// The metamodel rules over one edge

fn metamodel(value: &str) -> Metamodel {
    Metamodel::parse(value).expect("a well-formed metamodel")
}

/// model-toml.md "Metamodel": "only Syside-metamodel projects may depend
/// on Syside-metamodel projects" -- the rule is one-way.
#[test]
fn metamodel_pair_allows_an_omg_dependency_of_a_syside_project() {
    assert_eq!(
        check_metamodel_pair(
            (&"acme/a", Some(&metamodel(SYSIDE_SYSML_2025))),
            (&"acme/b", Some(&metamodel(OMG_SYSML_2025))),
        ),
        Ok(None)
    );
}

/// model-toml.md: "a `<date>` that differs between a project and one of
/// its dependencies is a warning".
#[test]
fn metamodel_pair_warns_on_a_different_release() {
    let mismatch = check_metamodel_pair(
        (&"acme/a", Some(&metamodel(OMG_SYSML_2025))),
        (&"acme/b", Some(&metamodel(OMG_KERML_2024))),
    )
    .expect("a differing release is not an error")
    .expect("a differing release is a warning");

    assert_eq!(
        mismatch.to_string(),
        "`acme/a` targets metamodel release `20250201` but its dependency `acme/b` \
         targets `20240201`"
    );
}

/// model-toml.md: "an OMG-metamodel project must not depend on a
/// Syside-metamodel project. This is an error".
#[test]
fn metamodel_pair_rejects_an_omg_project_depending_on_syside() {
    let error = check_metamodel_pair(
        (&"acme/a", Some(&metamodel(OMG_SYSML_2025))),
        (&"acme/b", Some(&metamodel(SYSIDE_SYSML_2025))),
    )
    .expect_err("an OMG project may not depend on a Syside one");

    assert_eq!(
        error.to_string(),
        "`acme/a` uses the OMG metamodel `https://www.omg.org/spec/SysML/20250201` and cannot \
         depend on `acme/b`, which uses the Syside metamodel \
         `https://www.sysand.com/spec/SysML/20250201`; only Syside-metamodel projects may \
         depend on Syside-metamodel projects"
    );
}

/// model-toml.md: a project without a `metamodel` "is treated as OMG with
/// an unknown date: the direction rule applies, the release warning does
/// not".
#[test]
fn metamodel_pair_of_a_project_without_a_metamodel() {
    let none = None;

    assert_eq!(
        check_metamodel_pair(
            (&"acme/a", none),
            (&"acme/b", Some(&metamodel(OMG_KERML_2024)))
        ),
        Ok(None)
    );

    let error = check_metamodel_pair(
        (&"acme/a", none),
        (&"acme/b", Some(&metamodel(SYSIDE_SYSML_2025))),
    )
    .expect_err("the direction rule applies to an unknown date");
    assert_eq!(
        error.to_string(),
        "`acme/a` uses no declared metamodel (OMG assumed) and cannot depend on `acme/b`, \
         which uses the Syside metamodel `https://www.sysand.com/spec/SysML/20250201`; \
         only Syside-metamodel projects may depend on Syside-metamodel projects"
    );
}

/// A metamodel this version does not recognize is validated against the
/// OMG schema, and is an OMG-metamodel project for the direction rule too.
#[test]
fn metamodel_pair_of_a_project_with_an_unrecognized_metamodel() {
    let unrecognized = metamodel("https://example.com/x");

    let error = check_metamodel_pair(
        (&"acme/a", Some(&unrecognized)),
        (&"acme/b", Some(&metamodel(SYSIDE_SYSML_2025))),
    )
    .expect_err("an unrecognized metamodel is OMG for the direction rule");
    assert!(
        error
            .to_string()
            .starts_with("`acme/a` uses the unrecognized metamodel `https://example.com/x` and"),
        "unexpected error: {error}"
    );

    assert_eq!(
        check_metamodel_pair(
            (&"acme/a", Some(&unrecognized)),
            (&"acme/b", Some(&metamodel(OMG_KERML_2024))),
        ),
        Ok(None)
    );
}

/// "Only the date is compared, across families and spec kinds."
#[test]
fn metamodel_pair_ignores_the_spec_kind() {
    assert_eq!(
        check_metamodel_pair(
            (&"acme/a", Some(&metamodel(OMG_SYSML_2025))),
            (&"acme/b", Some(&metamodel(OMG_KERML_2025))),
        ),
        Ok(None)
    );
    assert_eq!(
        check_metamodel_pair(
            (&"acme/a", Some(&metamodel(SYSIDE_SYSML_2025))),
            (&"acme/b", Some(&metamodel(OMG_KERML_2025))),
        ),
        Ok(None)
    );
}

/// Two Syside projects of one release: neither direction nor date has
/// anything to say.
#[test]
fn metamodel_pair_allows_a_syside_dependency_of_a_syside_project() {
    assert_eq!(
        check_metamodel_pair(
            (&"acme/a", Some(&metamodel(SYSIDE_SYSML_2025))),
            (&"acme/b", Some(&metamodel(SYSIDE_SYSML_2025))),
        ),
        Ok(None)
    );
}
