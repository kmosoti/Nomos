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

use crate::graph::{ConflictKey, Graph};

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

/// Greedy selection over `ready`, in the order given, against `held` keys,
/// with `reserved` Actions already counted against `capacity`.
pub fn select(
    graph: &Graph,
    ready: &[ResourcePath],
    held: &BTreeSet<ConflictKey>,
    reserved: usize,
    capacity: usize,
) -> Selection {
    let mut held = held.clone();
    let mut chosen = Vec::new();
    for resource in ready {
        if reserved + chosen.len() >= capacity {
            break;
        }
        let Some(vertex) = graph.vertex(resource) else {
            continue;
        };
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

    use super::select;
    use crate::graph::{ConflictKey, Graph, Vertex};
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

    /// Conflicting Actions are never selected together, and the earlier one
    /// in the order wins.
    #[test]
    fn conflicting_actions_are_never_selected_together() {
        let g = graph();
        let s = select(
            &g,
            &[r("/a"), r("/b"), r("/c"), r("/d")],
            &BTreeSet::new(),
            0,
            10,
        );
        assert_eq!(names(s.chosen()), ["/a", "/c", "/d"]);
        assert_eq!(*s.held(), keys(&["dpkg", "svc:y"]));
    }

    #[test]
    fn a_key_held_by_an_unsettled_effect_excludes_its_holders_conflicts() {
        let g = graph();
        let s = select(&g, &[r("/a"), r("/b"), r("/c")], &keys(&["dpkg"]), 1, 10);
        assert_eq!(names(s.chosen()), ["/c"]);
        assert_eq!(*s.held(), keys(&["dpkg", "svc:y"]));
    }

    #[test]
    fn capacity_counts_the_reserved_set() {
        let g = graph();
        let s = select(&g, &[r("/c"), r("/d")], &BTreeSet::new(), 2, 3);
        assert_eq!(names(s.chosen()), ["/c"]);
        let none = select(&g, &[r("/c"), r("/d")], &BTreeSet::new(), 3, 3);
        assert!(none.chosen().is_empty());
    }

    #[test]
    fn an_action_without_keys_conflicts_with_nothing() {
        let g = graph();
        let s = select(&g, &[r("/d"), r("/a")], &keys(&["dpkg"]), 1, 10);
        assert_eq!(names(s.chosen()), ["/d"]);
    }
}
