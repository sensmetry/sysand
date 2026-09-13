// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The checks that are about a *relation between two projects* rather
//! than about one project's manifest, and the graph they run over. A check
//! over a pair of projects belongs to no single loader, so it lives here,
//! apart from whatever builds the graph.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::{Debug, Display};
use std::hash::Hash;

use thiserror::Error;

/// A directed graph of projects, with an edge from a dependent to each of
/// its dependencies.
///
/// Nodes keep the order they were added in, which is the only tie-break
/// this module applies: a caller that wants ties broken in `members`
/// order adds them in that order, and a caller whose source is a hash map
/// adds them in a sorted order of its own, or its messages would differ
/// between runs.
#[derive(Clone, Debug, Default)]
pub struct DependencyGraph<N> {
    nodes: Vec<N>,
    index: HashMap<N, usize>,
    edges: Vec<Vec<usize>>,
}

impl<N: Clone + Debug + Display + Eq + Hash> DependencyGraph<N> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            index: HashMap::new(),
            edges: Vec::new(),
        }
    }

    /// Adds a node, or returns the index of the one that is already there.
    pub fn add_node(&mut self, node: N) -> usize {
        if let Some(index) = self.index.get(&node) {
            return *index;
        }
        let index = self.nodes.len();
        self.nodes.push(node.clone());
        self.index.insert(node, index);
        self.edges.push(Vec::new());
        index
    }

    /// Records that `dependent` depends on `dependency`.
    ///
    /// Returns `false`, having changed nothing, if either endpoint is not
    /// a node of this graph: a project's usages routinely name projects
    /// outside it (a workspace member depending on something that is not a
    /// member), and such a usage is not an edge of this graph. Adding the
    /// endpoint instead would put a project into an order it has no
    /// business being in.
    pub fn add_edge(&mut self, dependent: &N, dependency: &N) -> bool {
        let (Some(from), Some(to)) = (self.index.get(dependent), self.index.get(dependency)) else {
            return false;
        };
        let (from, to) = (*from, *to);
        let targets = &mut self.edges[from];
        if !targets.contains(&to) {
            targets.push(to);
        }
        true
    }

    pub fn nodes(&self) -> &[N] {
        &self.nodes
    }

    /// Every edge as a `(dependent, dependency)` pair, in the order the
    /// edges were added.
    pub fn edges(&self) -> impl Iterator<Item = (&N, &N)> {
        self.edges
            .iter()
            .enumerate()
            .flat_map(move |(from, targets)| {
                targets
                    .iter()
                    .map(move |to| (&self.nodes[from], &self.nodes[*to]))
            })
    }

    /// Every cycle in the graph, as its strongly connected component.
    ///
    /// A component of more than one node is a cycle, and so is a single
    /// node that depends on itself.
    pub fn cycles(&self) -> Vec<DependencyCycle<N>> {
        self.components()
            .into_iter()
            .filter(|component| self.is_cycle(component))
            .map(|component| self.cycle(&component, component[0]))
            .collect()
    }

    /// The projects with every one after the ones it depends on.
    ///
    /// A cycle has no such order, so its projects come out in the order
    /// they were added. Whether a cycle is allowed is the caller's to
    /// decide, through [`Self::cycles`].
    pub fn dependency_order(&self) -> Vec<&N> {
        self.components()
            .into_iter()
            .flatten()
            .map(|node| &self.nodes[node])
            .collect()
    }

    /// The strongly connected components, dependencies first, each in the
    /// order its nodes were added. Tarjan's algorithm, iteratively: a
    /// workspace is free to be a chain of thousands of members.
    fn components(&self) -> Vec<Vec<usize>> {
        struct State {
            index: Vec<Option<usize>>,
            lowlink: Vec<usize>,
            on_stack: Vec<bool>,
            stack: Vec<usize>,
            next_index: usize,
            components: Vec<Vec<usize>>,
        }

        let mut state = State {
            index: vec![None; self.nodes.len()],
            lowlink: vec![0; self.nodes.len()],
            on_stack: vec![false; self.nodes.len()],
            stack: Vec::new(),
            next_index: 0,
            components: Vec::new(),
        };

        for root in 0..self.nodes.len() {
            if state.index[root].is_some() {
                continue;
            }
            // Each frame is a node and how many of its edges are done.
            let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
            state.index[root] = Some(state.next_index);
            state.lowlink[root] = state.next_index;
            state.next_index += 1;
            state.stack.push(root);
            state.on_stack[root] = true;

            while let Some((node, edge)) = frames.pop() {
                if let Some(target) = self.edges[node].get(edge).copied() {
                    frames.push((node, edge + 1));
                    if state.index[target].is_none() {
                        state.index[target] = Some(state.next_index);
                        state.lowlink[target] = state.next_index;
                        state.next_index += 1;
                        state.stack.push(target);
                        state.on_stack[target] = true;
                        frames.push((target, 0));
                    } else if state.on_stack[target] {
                        state.lowlink[node] =
                            state.lowlink[node].min(state.index[target].expect("visited"));
                    }
                    continue;
                }

                if Some(state.lowlink[node]) == state.index[node] {
                    let mut component = Vec::new();
                    while let Some(member) = state.stack.pop() {
                        state.on_stack[member] = false;
                        component.push(member);
                        if member == node {
                            break;
                        }
                    }
                    // In the order the nodes were added
                    component.sort_unstable();
                    state.components.push(component);
                }

                if let Some((parent, _)) = frames.last().copied() {
                    state.lowlink[parent] = state.lowlink[parent].min(state.lowlink[node]);
                }
            }
        }

        state.components
    }

    fn is_cycle(&self, component: &[usize]) -> bool {
        component.len() > 1
            || component
                .first()
                .is_some_and(|node| self.edges[*node].contains(node))
    }

    /// A cycle through `start`, as a path that begins and ends there.
    ///
    /// Breadth-first, so it is the shortest cycle through `start` — not
    /// necessarily the shortest cycle in the component, and deliberately
    /// so: `start` is chosen by the caller to be a project the reader
    /// needs to see.
    fn cycle(&self, component: &[usize], start: usize) -> DependencyCycle<N> {
        let in_component: HashSet<usize> = component.iter().copied().collect();
        let mut previous: HashMap<usize, usize> = HashMap::new();
        let mut queue = VecDeque::from([start]);
        let mut end = None;

        'search: while let Some(current) = queue.pop_front() {
            for target in self.edges[current].iter().copied() {
                if !in_component.contains(&target) {
                    continue;
                }
                if target == start {
                    end = Some(current);
                    break 'search;
                }
                if let std::collections::hash_map::Entry::Vacant(entry) = previous.entry(target) {
                    entry.insert(current);
                    queue.push_back(target);
                }
            }
        }

        // The walk back from `end` through the breadth-first tree ends at
        // `start`, which is never a target of the tree and so has no
        // predecessor. A self-loop has `end == start` and one step.
        let mut path = Vec::new();
        if let Some(end) = end {
            let mut back = vec![end];
            let mut current = end;
            while let Some(step) = previous.get(&current).copied() {
                back.push(step);
                current = step;
            }
            back.reverse();
            path.extend(back.into_iter().map(|node| self.nodes[node].clone()));
        }
        path.push(self.nodes[start].clone());

        DependencyCycle {
            component: component
                .iter()
                .map(|node| self.nodes[*node].clone())
                .collect(),
            path,
        }
    }
}

/// A dependency cycle, and a path through it to show the reader.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error(
    "dependency cycle: {}; only projects that carry `stdlib` may form a cycle",
    render_path(path)
)]
pub struct DependencyCycle<N: Display> {
    /// Every project on the cycle, in the order the projects were added.
    pub component: Vec<N>,
    /// One cycle, starting and ending at the same project.
    pub path: Vec<N>,
}

fn render_path<N: Display>(path: &[N]) -> String {
    path.iter()
        .map(|node| format!("`{node}`"))
        .collect::<Vec<_>>()
        .join(" -> ")
}

#[cfg(test)]
#[path = "./graph_tests.rs"]
mod tests;
