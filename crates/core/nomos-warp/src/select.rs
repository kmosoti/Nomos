//! Conflict-aware selection over the reserved set
//! ([warp.md](../../../../docs/formal/warp.md#runnable-set), ADR 0009 §5).
//!
//! Independence in the graph is not independence in the operating system.
//! Each Action carries exclusive keys, and two Actions that share a key
//! never run at the same time. An Action holds its keys from dispatch until
//! its effect is Settled, past a timeout and past a satisfied re-observation,
//! so the reserved set is wider than the running set.
//!
//! Selection is greedy and deterministic: Ready vertices in topological
//! order, each taken when it fits the capacity, shares no key with anything
//! reserved or already chosen, and does not break a failure-domain budget
//! (N9, [`crate::budget`]). It optimizes nothing. One selection runs per
//! kernel step, so admissions are serialized and two cannot pass against
//! one snapshot.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use nomos_core::resource::ResourcePath;

use crate::budget::{Budgets, Node};
use crate::frontier::Frontier;
use crate::graph::{ConflictKey, Graph, VertexKind};

/// What is reserved before a selection: the keys held and the nodes
/// disrupted by every Action whose effect is not Settled or that is still
/// live, and how many such Actions there are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reserved {
    /// Keys held.
    pub keys: BTreeSet<ConflictKey>,
    /// Nodes disrupted.
    pub nodes: BTreeSet<Node>,
    /// Actions counted against capacity.
    pub count: usize,
}

/// What one selection pass chose, and what is reserved afterward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    chosen: Vec<ResourcePath>,
    held: BTreeSet<ConflictKey>,
    nodes: BTreeSet<Node>,
}

impl Selection {
    /// The Actions admitted, in the order they were considered.
    pub fn chosen(&self) -> &[ResourcePath] {
        &self.chosen
    }

    /// Every key reserved after this selection: the ones held before plus
    /// the chosen Actions' keys.
    pub fn held(&self) -> &BTreeSet<ConflictKey> {
        &self.held
    }

    /// Every node disrupted after this selection.
    pub fn nodes(&self) -> &BTreeSet<Node> {
        &self.nodes
    }
}

/// Greedy selection over the Ready vertices of `frontier`, in topological
/// order, against what is `reserved`, within `capacity` and `budgets`.
/// `frontier` is the frontier of `graph`.
///
/// It takes the frontier rather than a list, so that only a Ready Action can
/// be admitted: Runnable is a subset of Ready by construction, and a Blocked
/// or Waiting Action, or an anchor, cannot be handed to it (N4).
pub fn select(
    graph: &Graph,
    frontier: &Frontier,
    reserved: &Reserved,
    capacity: usize,
    budgets: &Budgets,
) -> Selection {
    let mut held = reserved.keys.clone();
    let mut nodes = reserved.nodes.clone();
    let mut chosen = Vec::new();
    for resource in frontier.ready() {
        if reserved.count + chosen.len() >= capacity {
            break;
        }
        let Some(vertex) = graph.vertex(resource) else {
            continue;
        };
        if vertex.kind() != VertexKind::Action {
            continue;
        }
        if vertex.keys().iter().any(|k| held.contains(k)) {
            continue;
        }
        if budgets.violated_by(&nodes, vertex.disrupts()) {
            continue;
        }
        held.extend(vertex.keys().iter().cloned());
        nodes.extend(vertex.disrupts().iter().cloned());
        chosen.push(resource.clone());
    }
    Selection {
        chosen,
        held,
        nodes,
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeSet;
    use alloc::vec;
    use alloc::vec::Vec;

    use alloc::collections::BTreeMap;

    use super::{Reserved, select};
    use crate::budget::{Budget, Budgets, Node};
    use crate::frontier::{Frontier, Outcome, Progress, frontier};
    use crate::graph::{ConflictKey, Edge, EdgeKind, Graph, Vertex};
    use nomos_core::resource::ResourcePath;

    fn r(text: &str) -> ResourcePath {
        ResourcePath::new(text).unwrap()
    }

    fn keys(names: &[&str]) -> BTreeSet<ConflictKey> {
        names.iter().map(|n| ConflictKey::new(n).unwrap()).collect()
    }

    fn graph() -> Graph {
        Graph::compile(
            vec![
                Vertex::action(r("/a"), keys(&["dpkg"])),
                Vertex::action(r("/b"), keys(&["dpkg", "svc:x"])),
                Vertex::action(r("/c"), keys(&["svc:y"])),
                Vertex::action(r("/d"), keys(&[])),
            ],
            vec![],
        )
        .unwrap()
    }

    fn held(names: &[&str], count: usize) -> Reserved {
        Reserved {
            keys: keys(names),
            nodes: BTreeSet::new(),
            count,
        }
    }

    fn nodes(names: &[&str]) -> BTreeSet<Node> {
        names.iter().map(|n| Node::new(n).unwrap()).collect()
    }

    /// Four independent restarts, each disrupting one node of a three-node
    /// rack that may lose one at a time.
    fn restarts() -> (Graph, Budgets) {
        let g = Graph::compile(
            vec![
                Vertex::action(r("/a"), keys(&[])).disrupting(nodes(&["n1"])),
                Vertex::action(r("/b"), keys(&[])).disrupting(nodes(&["n2"])),
                Vertex::action(r("/c"), keys(&[])).disrupting(nodes(&["n3"])),
                Vertex::action(r("/d"), keys(&[])),
            ],
            vec![],
        )
        .unwrap();
        let rack = Budget::new(nodes(&["n1", "n2", "n3"]), 1);
        (g, Budgets::new(vec![rack], BTreeSet::new(), true))
    }

    /// N9 within one selection: two disruptive Actions against one snapshot
    /// do not both pass; the second sees the first as chosen.
    #[test]
    fn one_selection_admits_within_the_budget() {
        let (g, budgets) = restarts();
        let f = ready_only(&g, &["/a", "/b", "/c", "/d"]);
        let s = select(&g, &f, &held(&[], 0), 10, &budgets);
        assert_eq!(names(s.chosen()), ["/a", "/d"]);
        assert_eq!(*s.nodes(), nodes(&["n1"]));
    }

    /// N9 across selections, and semantic mutant `SM-TRANSITION-007`: a node
    /// disrupted by a reserved Action, one whose effect is not Settled,
    /// counts against the budget of the next selection.
    #[test]
    fn reserved_disruption_counts_against_the_budget() {
        let (g, budgets) = restarts();
        let f = ready_only(&g, &["/b", "/c", "/d"]);
        let reserved = Reserved {
            keys: BTreeSet::new(),
            nodes: nodes(&["n1"]),
            count: 1,
        };
        let s = select(&g, &f, &reserved, 10, &budgets);
        assert_eq!(names(s.chosen()), ["/d"]);
    }

    /// `budget-not-world`: a node already unavailable in the snapshot counts,
    /// and a stale snapshot admits nothing disruptive.
    #[test]
    fn unavailable_and_stale_snapshots_hold_back_disruption() {
        let (g, _) = restarts();
        let rack = || Budget::new(nodes(&["n1", "n2", "n3"]), 1);
        let f = ready_only(&g, &["/a", "/b", "/c", "/d"]);
        let down = Budgets::new(vec![rack()], nodes(&["n3"]), true);
        let s = select(&g, &f, &held(&[], 0), 10, &down);
        assert_eq!(names(s.chosen()), ["/c", "/d"]);
        let stale = Budgets::new(vec![rack()], BTreeSet::new(), false);
        let s = select(&g, &f, &held(&[], 0), 10, &stale);
        assert_eq!(names(s.chosen()), ["/d"]);
    }

    fn names(paths: &[ResourcePath]) -> Vec<&str> {
        paths.iter().map(ResourcePath::as_str).collect()
    }

    /// The frontier with exactly `ready` pending; every other Action is Running.
    fn ready_only(g: &Graph, ready: &[&str]) -> Frontier {
        let progress: BTreeMap<ResourcePath, Progress> = g
            .vertices()
            .filter(|v| !ready.contains(&v.resource().as_str()))
            .map(|v| (v.resource().clone(), Progress::Running))
            .collect();
        frontier(g, &progress)
    }

    /// Conflicting Actions are never selected together, and the earlier one
    /// in the order wins.
    #[test]
    fn conflicting_actions_are_never_selected_together() {
        let g = graph();
        let f = ready_only(&g, &["/a", "/b", "/c", "/d"]);
        let s = select(&g, &f, &held(&[], 0), 10, &Budgets::none());
        assert_eq!(names(s.chosen()), ["/a", "/c", "/d"]);
        assert_eq!(*s.held(), keys(&["dpkg", "svc:y"]));
    }

    #[test]
    fn a_key_held_by_an_unsettled_effect_excludes_its_holders_conflicts() {
        let g = graph();
        let f = ready_only(&g, &["/a", "/b", "/c"]);
        let s = select(&g, &f, &held(&["dpkg"], 1), 10, &Budgets::none());
        assert_eq!(names(s.chosen()), ["/c"]);
        assert_eq!(*s.held(), keys(&["dpkg", "svc:y"]));
    }

    #[test]
    fn capacity_counts_the_reserved_set() {
        let g = graph();
        let f = ready_only(&g, &["/c", "/d"]);
        let s = select(&g, &f, &held(&[], 2), 3, &Budgets::none());
        assert_eq!(names(s.chosen()), ["/c"]);
        let none = select(&g, &f, &held(&[], 3), 3, &Budgets::none());
        assert!(none.chosen().is_empty());
    }

    #[test]
    fn an_action_without_keys_conflicts_with_nothing() {
        let g = graph();
        let f = ready_only(&g, &["/a", "/d"]);
        let s = select(&g, &f, &held(&["dpkg"], 1), 10, &Budgets::none());
        assert_eq!(names(s.chosen()), ["/d"]);
    }

    /// Only Ready Actions are admitted: a Blocked or Waiting Action and an
    /// anchor never are, whatever keys and capacity allow (N4).
    #[test]
    fn only_ready_actions_are_selected() {
        let g = Graph::compile(
            vec![
                Vertex::action(r("/failed"), keys(&[])),
                Vertex::action(r("/blocked"), keys(&[])),
                Vertex::action(r("/running"), keys(&[])),
                Vertex::action(r("/waiting"), keys(&[])),
                Vertex::anchor(r("/anchor")),
                Vertex::action(r("/ready"), keys(&[])),
            ],
            vec![
                Edge::new(r("/failed"), r("/blocked"), EdgeKind::Requires),
                Edge::new(r("/running"), r("/waiting"), EdgeKind::After),
                Edge::new(r("/anchor"), r("/ready"), EdgeKind::Requires),
            ],
        )
        .unwrap();
        let progress: BTreeMap<ResourcePath, Progress> = [
            (r("/failed"), Progress::Done(Outcome::Failed)),
            (r("/running"), Progress::Running),
        ]
        .into();
        let f = frontier(&g, &progress);
        let s = select(&g, &f, &held(&[], 0), 100, &Budgets::none());
        assert_eq!(names(s.chosen()), ["/ready"]);
    }
}
