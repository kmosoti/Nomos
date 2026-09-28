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
//! order, each taken when it fits the capacity and shares no key with
//! anything reserved or already chosen. It optimizes nothing. Failure-domain
//! budgets (N9) are a Loom-level policy the transition kernel adds as a
//! predicate here; this function has a slot for it and no policy yet.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use nomos_core::resource::ResourcePath;

use crate::frontier::Frontier;
use crate::graph::{ConflictKey, Graph, VertexKind};

/// What one selection pass chose, and the keys held afterward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    chosen: Vec<ResourcePath>,
    held: BTreeSet<ConflictKey>,
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
}

/// Greedy selection over the Ready vertices of `frontier`, in topological
/// order, against `held` keys, with `reserved` Actions already counted
/// against `capacity`. `frontier` is the frontier of `graph`.
///
/// It takes the frontier rather than a list, so that only a Ready Action can
/// be admitted: Runnable is a subset of Ready by construction, and a Blocked
/// or Waiting Action, or an anchor, cannot be handed to it (N4).
pub fn select(
    graph: &Graph,
    frontier: &Frontier,
    held: &BTreeSet<ConflictKey>,
    reserved: usize,
    capacity: usize,
) -> Selection {
    let mut held = held.clone();
    let mut chosen = Vec::new();
    for resource in frontier.ready() {
        if reserved + chosen.len() >= capacity {
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
        held.extend(vertex.keys().iter().cloned());
        chosen.push(resource.clone());
    }
    Selection { chosen, held }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeSet;
    use alloc::vec;
    use alloc::vec::Vec;

    use alloc::collections::BTreeMap;

    use super::select;
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
        let s = select(&g, &f, &BTreeSet::new(), 0, 10);
        assert_eq!(names(s.chosen()), ["/a", "/c", "/d"]);
        assert_eq!(*s.held(), keys(&["dpkg", "svc:y"]));
    }

    #[test]
    fn a_key_held_by_an_unsettled_effect_excludes_its_holders_conflicts() {
        let g = graph();
        let f = ready_only(&g, &["/a", "/b", "/c"]);
        let s = select(&g, &f, &keys(&["dpkg"]), 1, 10);
        assert_eq!(names(s.chosen()), ["/c"]);
        assert_eq!(*s.held(), keys(&["dpkg", "svc:y"]));
    }

    #[test]
    fn capacity_counts_the_reserved_set() {
        let g = graph();
        let f = ready_only(&g, &["/c", "/d"]);
        let s = select(&g, &f, &BTreeSet::new(), 2, 3);
        assert_eq!(names(s.chosen()), ["/c"]);
        let none = select(&g, &f, &BTreeSet::new(), 3, 3);
        assert!(none.chosen().is_empty());
    }

    #[test]
    fn an_action_without_keys_conflicts_with_nothing() {
        let g = graph();
        let f = ready_only(&g, &["/a", "/d"]);
        let s = select(&g, &f, &keys(&["dpkg"]), 1, 10);
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
        let s = select(&g, &f, &BTreeSet::new(), 0, 100);
        assert_eq!(names(s.chosen()), ["/ready"]);
    }
}
