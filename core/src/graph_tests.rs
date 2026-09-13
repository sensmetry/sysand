// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::graph::DependencyGraph;

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
fn edges_are_listed_in_insertion_order() {
    let graph = graph(&["a", "b", "c"], &[("a", "b"), ("a", "c"), ("b", "c")]);

    assert_eq!(
        graph.edges().collect::<Vec<_>>(),
        [
            (&"a".to_owned(), &"b".to_owned()),
            (&"a".to_owned(), &"c".to_owned()),
            (&"b".to_owned(), &"c".to_owned()),
        ]
    );
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
    assert_eq!(graph.edges().count(), 1);
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
