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
    graph
        .build_order(|_| false)
        .expect("no cycle")
        .into_iter()
        .cloned()
        .collect()
}

/// workspaces.md "Build output": "A workspace builds its members in
/// dependency order: a topological sort of the members' `dir` usages of
/// each other."
#[test]
fn build_order_is_dependencies_first() {
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

/// workspaces.md: "A dependency cycle is a hard error listing the cycle".
#[test]
fn build_order_rejects_a_cycle() {
    let graph = graph(
        &["acme/a", "acme/b", "acme/c"],
        &[
            ("acme/a", "acme/b"),
            ("acme/b", "acme/a"),
            ("acme/c", "acme/a"),
        ],
    );

    let cycle = graph
        .build_order(|_| false)
        .expect_err("a <-> b is a cycle");
    assert_eq!(cycle.component, ["acme/a", "acme/b"]);
    assert_eq!(cycle.path, ["acme/a", "acme/b", "acme/a"]);
    assert_eq!(
        cycle.to_string(),
        "dependency cycle: `acme/a` -> `acme/b` -> `acme/a`; \
         only projects that carry `stdlib` may form a cycle"
    );
}

/// workspaces.md: a cycle is an error "unless every project on it carries
/// the `stdlib` key ...; such a cycle is built in `members` order".
#[test]
fn build_order_rejects_a_cycle_unless_every_project_allows_it() {
    let graph = graph(&["a", "b", "c"], &[("a", "b"), ("b", "a"), ("c", "a")]);

    graph
        .build_order(|node| node == "a")
        .expect_err("`b` does not allow the cycle");
    assert_eq!(
        graph
            .build_order(|node| node != "c")
            .expect("every project on the cycle allows it")
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
}

/// The witness names a project that is the reason the cycle is refused,
/// not just any project on it.
#[test]
fn build_order_names_a_project_that_forbids_the_cycle() {
    let graph = graph(
        &["a", "b", "c"],
        &[("a", "b"), ("b", "a"), ("a", "c"), ("c", "a")],
    );

    let cycle = graph
        .build_order(|node| node != "c")
        .expect_err("`c` does not allow the cycle");
    assert_eq!(cycle.component, ["a", "b", "c"]);
    assert!(
        cycle.path.iter().any(|node| node == "c"),
        "the witness `{:?}` should name `c`",
        cycle.path
    );
    assert_eq!(cycle.path, ["c", "a", "c"]);
}

#[test]
fn build_order_treats_a_self_usage_as_a_cycle() {
    let graph = graph(&["a"], &[("a", "a")]);

    let cycle = graph.build_order(|_| false).expect_err("a uses itself");
    assert_eq!(cycle.component, ["a"]);
    assert_eq!(cycle.path, ["a", "a"]);

    graph
        .build_order(|_| true)
        .expect("a self-usage is allowed when the project allows it");
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
fn dependents_of_is_transitive() {
    let graph = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);

    assert_eq!(graph.dependents_of(&"c".to_owned()), ["b", "a"]);
    assert_eq!(graph.dependents_of(&"b".to_owned()), ["a"]);
    assert_eq!(
        graph.dependents_of(&"a".to_owned()),
        [] as [&std::string::String; 0]
    );
    assert_eq!(
        graph.dependents_of(&"outsider".to_owned()),
        [] as [&std::string::String; 0]
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
fn build_order_handles_a_long_chain_without_overflowing() {
    const LENGTH: usize = 50_000;

    let mut graph = DependencyGraph::new();
    for node in 0..LENGTH {
        graph.add_node(node);
    }
    for node in 1..LENGTH {
        assert!(graph.add_edge(&(node - 1), &node));
    }

    let order = graph.build_order(|_| false).expect("a chain has no cycle");
    assert_eq!(order.len(), LENGTH);
    assert_eq!(*order[0], LENGTH - 1);
    assert_eq!(*order[LENGTH - 1], 0);
}
