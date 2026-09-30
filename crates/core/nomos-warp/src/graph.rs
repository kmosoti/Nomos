//! The Warp graph: vertices from Assessments, edges from the Canon, one
//! deterministic order, and one explicit witness per cycle
//! ([warp.md](../../../../docs/formal/warp.md), ADR 0009).
//!
//! A vertex is one resource. A resource with a Variance gets an Action
//! vertex; a Satisfied resource gets a satisfaction anchor, Succeeded on
//! entry and unchanged; an Indeterminate resource gets an Indeterminate
//! anchor, terminal on entry and not Succeeded. Every resource an edge names
//! must have a vertex, so an edge to a resource with no Assessment is a
//! compilation error rather than a dangling reference.
//!
//! Compilation runs Kahn's algorithm with an ordered queue, so the order is
//! a function of the graph alone (N12). When vertices remain, the remainder
//! is partitioned into strongly connected components; each cyclic component
//! yields one witness cycle, and every acyclic remainder vertex is reported
//! as blocked by a cycle upstream of it. The recursion in the component
//! search is bounded by the vertex count of a Canon.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use nomos_core::assessment::Assessment;
use nomos_core::resource::ResourceKey;

use crate::budget::Node;

/// The three edge kinds of spec §14. An edge `(source, target)` means the
/// source constrains the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum EdgeKind {
    /// The target may start only after the source Succeeded.
    Requires,
    /// The target is ordered after the source, whatever its outcome.
    After,
    /// The target has a reason to run only if a source produced a verified change.
    OnChange,
}

/// One dependency edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Edge {
    source: ResourceKey,
    target: ResourceKey,
    kind: EdgeKind,
}

impl Edge {
    /// An edge from `source` to `target`: the source constrains the target.
    pub fn new(source: ResourceKey, target: ResourceKey, kind: EdgeKind) -> Self {
        Edge {
            source,
            target,
            kind,
        }
    }

    /// The constraining resource.
    pub fn source(&self) -> &ResourceKey {
        &self.source
    }

    /// The constrained resource.
    pub fn target(&self) -> &ResourceKey {
        &self.target
    }

    /// The edge kind.
    pub fn kind(&self) -> EdgeKind {
        self.kind
    }
}

/// An exclusive key an Action holds while its effect is unsettled, for
/// example `file:/etc/hosts` or `package-manager:dpkg`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConflictKey(String);

impl ConflictKey {
    /// A key from non-empty text.
    pub fn new(text: &str) -> Option<Self> {
        (!text.is_empty()).then(|| ConflictKey(String::from(text)))
    }

    /// The key text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a vertex is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VertexKind {
    /// An Action: the resource has a Variance or an Obligation, or is the
    /// `on_change` target of an Action, and something may run.
    Action,
    /// A satisfaction anchor: Succeeded on entry, unchanged, no operation.
    Anchor,
    /// An Indeterminate anchor: terminal on entry with outcome Indeterminate.
    IndeterminateAnchor,
}

/// One vertex: a resource, what it is, the keys its Action holds, whether
/// an Obligation is owed on it, whether it has a Variance of its own, and
/// the nodes its Action would disrupt.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Vertex {
    resource: ResourceKey,
    kind: VertexKind,
    keys: BTreeSet<ConflictKey>,
    owed: bool,
    varies: bool,
    disrupts: BTreeSet<Node>,
}

impl Vertex {
    /// An Action vertex holding `keys`.
    pub fn action(resource: ResourceKey, keys: BTreeSet<ConflictKey>) -> Self {
        Vertex {
            resource,
            kind: VertexKind::Action,
            keys,
            owed: false,
            varies: false,
            disrupts: BTreeSet::new(),
        }
    }

    /// An owed Action vertex: an Obligation is pending on the resource, so
    /// the Action runs even when its `on_change` group is Disabled
    /// (ADR 0009 note). It still waits for its triggers and is still
    /// Blocked by one that is not met.
    pub fn owed(resource: ResourceKey, keys: BTreeSet<ConflictKey>) -> Self {
        Vertex {
            owed: true,
            ..Vertex::action(resource, keys)
        }
    }

    /// An Action vertex for a resource with a Variance of its own. Like an
    /// owed vertex, it has a reason to run that is not its `on_change`
    /// sources, so a Disabled group makes it Ready (warp.md, owed and
    /// varying vertices). It still waits for its triggers and is still
    /// Blocked by one that is not met.
    pub fn varying(resource: ResourceKey, keys: BTreeSet<ConflictKey>) -> Self {
        Vertex {
            varies: true,
            ..Vertex::action(resource, keys)
        }
    }

    /// A satisfaction anchor. Anchors hold no keys: they mutate nothing.
    pub fn anchor(resource: ResourceKey) -> Self {
        Vertex {
            resource,
            kind: VertexKind::Anchor,
            keys: BTreeSet::new(),
            owed: false,
            varies: false,
            disrupts: BTreeSet::new(),
        }
    }

    /// An Indeterminate anchor.
    pub fn indeterminate_anchor(resource: ResourceKey) -> Self {
        Vertex {
            resource,
            kind: VertexKind::IndeterminateAnchor,
            keys: BTreeSet::new(),
            owed: false,
            varies: false,
            disrupts: BTreeSet::new(),
        }
    }

    /// The same vertex, whose Action would make `nodes` unavailable. An
    /// anchor runs nothing, so it disrupts nothing and keeps no nodes.
    pub fn disrupting(mut self, nodes: BTreeSet<Node>) -> Self {
        if self.kind == VertexKind::Action {
            self.disrupts = nodes;
        }
        self
    }

    /// The vertex a resource gets from its Assessment: a varying Action for
    /// a Variance, an anchor for Satisfied, an Indeterminate anchor otherwise.
    pub fn from_assessment(
        resource: ResourceKey,
        assessment: &Assessment,
        keys: BTreeSet<ConflictKey>,
    ) -> Self {
        match assessment {
            Assessment::Variance(_) => Vertex::varying(resource, keys),
            Assessment::Satisfied => Vertex::anchor(resource),
            Assessment::Indeterminate(_) => Vertex::indeterminate_anchor(resource),
        }
    }

    /// The resource.
    pub fn resource(&self) -> &ResourceKey {
        &self.resource
    }

    /// What the vertex is.
    pub fn kind(&self) -> VertexKind {
        self.kind
    }

    /// The keys the Action holds; empty for an anchor.
    pub fn keys(&self) -> &BTreeSet<ConflictKey> {
        &self.keys
    }

    /// Whether an Obligation is owed on the resource.
    pub fn is_owed(&self) -> bool {
        self.owed
    }

    /// Whether the resource has a Variance of its own.
    pub fn varies(&self) -> bool {
        self.varies
    }

    /// Whether the Action has a reason to run besides its `on_change`
    /// sources: an owed Obligation or a Variance of its own.
    pub fn has_own_reason(&self) -> bool {
        self.owed || self.varies
    }

    /// The nodes the Action would make unavailable; empty for an anchor.
    pub fn disrupts(&self) -> &BTreeSet<Node> {
        &self.disrupts
    }
}

/// Why a set of vertices and edges is not a Plan graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    /// Two vertices name one resource.
    DuplicateVertex(ResourceKey),
    /// An edge names a resource with no vertex.
    UnknownResource {
        /// The edge that names it.
        edge: Edge,
        /// The resource with no vertex.
        missing: ResourceKey,
    },
    /// The edges contain a cycle.
    Cycle {
        /// One closed walk per cyclic component, each starting at the
        /// component's smallest resource; every listed vertex has an edge to
        /// the next and the last has an edge back to the first.
        witnesses: Vec<Vec<ResourceKey>>,
        /// Acyclic vertices downstream of a cycle, which can never be ordered.
        blocked: Vec<ResourceKey>,
    },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::DuplicateVertex(r) => write!(f, "two vertices name {r}"),
            CompileError::UnknownResource { edge, missing } => write!(
                f,
                "edge {} -> {} ({:?}) names {missing}, which has no vertex",
                edge.source, edge.target, edge.kind
            ),
            CompileError::Cycle { witnesses, blocked } => write!(
                f,
                "{} cycle(s); {} vertex(es) blocked downstream",
                witnesses.len(),
                blocked.len()
            ),
        }
    }
}

/// A compiled Plan graph: acyclic, with one topological order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    vertices: BTreeMap<ResourceKey, Vertex>,
    edges: Vec<Edge>,
    order: Vec<ResourceKey>,
}

impl Graph {
    /// Compiles vertices and edges into a graph, or says why it cannot.
    pub fn compile(vertices: Vec<Vertex>, edges: Vec<Edge>) -> Result<Self, CompileError> {
        let mut by_resource: BTreeMap<ResourceKey, Vertex> = BTreeMap::new();
        for vertex in vertices {
            let resource = vertex.resource.clone();
            if by_resource.insert(resource.clone(), vertex).is_some() {
                return Err(CompileError::DuplicateVertex(resource));
            }
        }
        // Edges are a set: order and repetition carry no meaning.
        let edges: BTreeSet<Edge> = edges.into_iter().collect();
        for edge in &edges {
            for end in [&edge.source, &edge.target] {
                if !by_resource.contains_key(end) {
                    return Err(CompileError::UnknownResource {
                        edge: edge.clone(),
                        missing: end.clone(),
                    });
                }
            }
        }
        let edges: Vec<Edge> = edges.into_iter().collect();
        let order = kahn(&by_resource, &edges);
        if order.len() < by_resource.len() {
            let remainder: BTreeSet<&ResourceKey> =
                by_resource.keys().filter(|r| !order.contains(r)).collect();
            return Err(cycle_diagnostic(&remainder, &edges));
        }
        Ok(Graph {
            vertices: by_resource,
            edges,
            order,
        })
    }

    /// The topological order: every edge's source precedes its target.
    pub fn order(&self) -> &[ResourceKey] {
        &self.order
    }

    /// The vertex for a resource.
    pub fn vertex(&self, resource: &ResourceKey) -> Option<&Vertex> {
        self.vertices.get(resource)
    }

    /// Every vertex, by resource.
    pub fn vertices(&self) -> impl Iterator<Item = &Vertex> {
        self.vertices.values()
    }

    /// Every edge, sorted.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// The edges into `target`.
    pub fn edges_into<'a>(&'a self, target: &'a ResourceKey) -> impl Iterator<Item = &'a Edge> {
        self.edges.iter().filter(move |e| &e.target == target)
    }
}

/// Kahn's algorithm with an ordered queue: the output is a function of the
/// graph alone. Returns the vertices ordered; fewer than all means a cycle.
fn kahn(vertices: &BTreeMap<ResourceKey, Vertex>, edges: &[Edge]) -> Vec<ResourceKey> {
    let mut indegree: BTreeMap<&ResourceKey, usize> =
        vertices.keys().map(|r| (r, 0usize)).collect();
    for edge in edges {
        if let Some(n) = indegree.get_mut(&edge.target) {
            *n += 1;
        }
    }
    let mut queue: BTreeSet<&ResourceKey> = indegree
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(r, _)| *r)
        .collect();
    let mut order = Vec::with_capacity(vertices.len());
    while let Some(next) = queue.pop_first() {
        order.push(next.clone());
        for edge in edges.iter().filter(|e| &e.source == next) {
            if let Some(n) = indegree.get_mut(&edge.target) {
                *n -= 1;
                if *n == 0 {
                    queue.insert(&edge.target);
                }
            }
        }
    }
    order
}

/// Strongly connected components of the remainder, one witness per cyclic
/// component, and the acyclic remainder as blocked.
fn cycle_diagnostic(remainder: &BTreeSet<&ResourceKey>, edges: &[Edge]) -> CompileError {
    let successors = |r: &ResourceKey| -> Vec<&ResourceKey> {
        edges
            .iter()
            .filter(|e| &e.source == r && remainder.contains(&e.target))
            .map(|e| &e.target)
            .collect()
    };
    let components = strongly_connected(remainder, &successors);
    let mut witnesses = Vec::new();
    let mut blocked = Vec::new();
    for component in components {
        let cyclic = component.len() > 1 || component.iter().any(|r| successors(r).contains(r));
        if cyclic {
            if let Some(witness) = witness(&component, &successors) {
                witnesses.push(witness);
            }
        } else {
            blocked.extend(component.into_iter().cloned());
        }
    }
    witnesses.sort();
    blocked.sort();
    CompileError::Cycle { witnesses, blocked }
}

/// Kosaraju's algorithm over the remainder. Components are returned as
/// sorted sets, in order of their smallest vertex.
fn strongly_connected<'a>(
    vertices: &BTreeSet<&'a ResourceKey>,
    successors: &dyn Fn(&ResourceKey) -> Vec<&'a ResourceKey>,
) -> Vec<BTreeSet<&'a ResourceKey>> {
    // First pass: finishing order.
    let mut finished: Vec<&ResourceKey> = Vec::new();
    let mut seen: BTreeSet<&ResourceKey> = BTreeSet::new();
    for start in vertices {
        finish_order(start, successors, &mut seen, &mut finished);
    }
    // Second pass: on the transposed graph, in reverse finishing order.
    let predecessors = |r: &ResourceKey| -> Vec<&'a ResourceKey> {
        vertices
            .iter()
            .filter(|v| successors(v).contains(&r))
            .copied()
            .collect()
    };
    let mut assigned: BTreeSet<&ResourceKey> = BTreeSet::new();
    let mut components: Vec<BTreeSet<&ResourceKey>> = Vec::new();
    for start in finished.iter().rev() {
        if assigned.contains(start) {
            continue;
        }
        let mut component = BTreeSet::new();
        let mut stack = alloc::vec![*start];
        while let Some(v) = stack.pop() {
            if !assigned.insert(v) {
                continue;
            }
            component.insert(v);
            for p in predecessors(v) {
                if !assigned.contains(p) {
                    stack.push(p);
                }
            }
        }
        components.push(component);
    }
    components.sort_by(|a, b| a.iter().next().cmp(&b.iter().next()));
    components
}

fn finish_order<'a>(
    start: &'a ResourceKey,
    successors: &dyn Fn(&ResourceKey) -> Vec<&'a ResourceKey>,
    seen: &mut BTreeSet<&'a ResourceKey>,
    finished: &mut Vec<&'a ResourceKey>,
) {
    if !seen.insert(start) {
        return;
    }
    for next in successors(start) {
        finish_order(next, successors, seen, finished);
    }
    finished.push(start);
}

/// The shortest cycle through the component's smallest vertex, by
/// breadth-first search inside the component.
fn witness<'a>(
    component: &BTreeSet<&'a ResourceKey>,
    successors: &dyn Fn(&ResourceKey) -> Vec<&'a ResourceKey>,
) -> Option<Vec<ResourceKey>> {
    let start = *component.iter().next()?;
    let mut parent: BTreeMap<&ResourceKey, &ResourceKey> = BTreeMap::new();
    let mut queue: VecDeque<&ResourceKey> = VecDeque::from([start]);
    let mut closing: Option<&ResourceKey> = None;
    'search: while let Some(v) = queue.pop_front() {
        for next in successors(v) {
            if !component.contains(next) {
                continue;
            }
            if next == start {
                closing = Some(v);
                break 'search;
            }
            if !parent.contains_key(next) {
                parent.insert(next, v);
                queue.push_back(next);
            }
        }
    }
    let mut walk = Vec::new();
    let mut cursor = closing?;
    while cursor != start {
        walk.push(cursor.clone());
        cursor = parent.get(cursor)?;
    }
    walk.push(start.clone());
    walk.reverse();
    Some(walk)
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeSet;
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{CompileError, Edge, EdgeKind, Graph, Vertex, VertexKind};
    use nomos_core::assessment::{Assessment, Reason, Variance};
    use nomos_core::resource::ResourceKey;

    fn r(text: &str) -> ResourceKey {
        ResourceKey::File(nomos_core::resource::ResourcePath::new(text).unwrap())
    }

    fn action(text: &str) -> Vertex {
        Vertex::action(r(text), BTreeSet::new())
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge::new(r(from), r(to), kind)
    }

    fn names(paths: &[ResourceKey]) -> Vec<&str> {
        paths.iter().map(ResourceKey::name).collect()
    }

    #[test]
    fn a_vertex_follows_its_assessment() {
        let keys = BTreeSet::new();
        assert_eq!(
            Vertex::from_assessment(
                r("/a"),
                &Assessment::Variance(Variance::Missing),
                keys.clone()
            )
            .kind(),
            VertexKind::Action
        );
        assert_eq!(
            Vertex::from_assessment(r("/a"), &Assessment::Satisfied, keys.clone()).kind(),
            VertexKind::Anchor
        );
        assert_eq!(
            Vertex::from_assessment(
                r("/a"),
                &Assessment::Indeterminate(Reason::NoObservation),
                keys
            )
            .kind(),
            VertexKind::IndeterminateAnchor
        );
    }

    #[test]
    fn the_order_respects_every_edge_and_breaks_ties_by_resource() {
        let graph = Graph::compile(
            vec![action("/c"), action("/a"), action("/b"), action("/d")],
            vec![
                edge("/c", "/a", EdgeKind::Requires),
                edge("/c", "/b", EdgeKind::After),
                edge("/b", "/d", EdgeKind::OnChange),
            ],
        )
        .unwrap();
        assert_eq!(names(graph.order()), ["/c", "/a", "/b", "/d"]);
    }

    #[test]
    fn the_order_is_a_function_of_the_graph_not_of_insertion_order() {
        let vertices = vec![action("/a"), action("/b"), action("/c")];
        let edges = vec![
            edge("/a", "/c", EdgeKind::Requires),
            edge("/b", "/c", EdgeKind::After),
        ];
        let forward = Graph::compile(vertices.clone(), edges.clone()).unwrap();
        let mut rv = vertices;
        rv.reverse();
        let mut re = edges;
        re.reverse();
        re.push(edge("/a", "/c", EdgeKind::Requires));
        let backward = Graph::compile(rv, re).unwrap();
        assert_eq!(forward, backward);
    }

    #[test]
    fn a_requires_cycle_is_rejected_with_one_witness() {
        let err = Graph::compile(
            vec![action("/a"), action("/b"), action("/c")],
            vec![
                edge("/a", "/b", EdgeKind::Requires),
                edge("/b", "/c", EdgeKind::Requires),
                edge("/c", "/a", EdgeKind::Requires),
            ],
        )
        .unwrap_err();
        match err {
            CompileError::Cycle { witnesses, blocked } => {
                assert_eq!(witnesses.len(), 1);
                assert_eq!(names(&witnesses[0]), ["/a", "/b", "/c"]);
                assert!(blocked.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    /// The cycle-witness case: $A \leftrightarrow B$, $B \to C$ yields exactly
    /// one witness, and $C$ is reported blocked, not cyclic.
    #[test]
    fn a_two_cycle_with_a_dependent_yields_one_witness_and_one_blocked_vertex() {
        let err = Graph::compile(
            vec![action("/a"), action("/b"), action("/c")],
            vec![
                edge("/a", "/b", EdgeKind::Requires),
                edge("/b", "/a", EdgeKind::Requires),
                edge("/b", "/c", EdgeKind::Requires),
            ],
        )
        .unwrap_err();
        match err {
            CompileError::Cycle { witnesses, blocked } => {
                assert_eq!(witnesses.len(), 1);
                assert_eq!(names(&witnesses[0]), ["/a", "/b"]);
                assert_eq!(names(&blocked), ["/c"]);
            }
            other => panic!("{other:?}"),
        }
    }

    /// Well-formedness is over the whole edge set: an `after` cycle is a
    /// cycle (semantic mutant `SM-WARP-003`).
    #[test]
    fn a_cycle_through_after_edges_is_rejected() {
        let err = Graph::compile(
            vec![action("/a"), action("/b")],
            vec![
                edge("/a", "/b", EdgeKind::Requires),
                edge("/b", "/a", EdgeKind::After),
            ],
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::Cycle { ref witnesses, .. } if witnesses.len() == 1));
        let err = Graph::compile(
            vec![action("/a"), action("/b")],
            vec![
                edge("/a", "/b", EdgeKind::OnChange),
                edge("/b", "/a", EdgeKind::After),
            ],
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::Cycle { .. }));
    }

    #[test]
    fn a_self_loop_is_a_cycle_of_one() {
        let err = Graph::compile(
            vec![action("/a"), action("/b")],
            vec![
                edge("/a", "/a", EdgeKind::After),
                edge("/a", "/b", EdgeKind::Requires),
            ],
        )
        .unwrap_err();
        match err {
            CompileError::Cycle { witnesses, blocked } => {
                assert_eq!(witnesses, vec![vec![r("/a")]]);
                assert_eq!(names(&blocked), ["/b"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn two_separate_cycles_yield_two_witnesses() {
        let err = Graph::compile(
            vec![action("/a"), action("/b"), action("/c"), action("/d")],
            vec![
                edge("/a", "/b", EdgeKind::Requires),
                edge("/b", "/a", EdgeKind::Requires),
                edge("/c", "/d", EdgeKind::After),
                edge("/d", "/c", EdgeKind::OnChange),
            ],
        )
        .unwrap_err();
        match err {
            CompileError::Cycle { witnesses, .. } => {
                assert_eq!(witnesses.len(), 2);
                assert_eq!(names(&witnesses[0]), ["/a", "/b"]);
                assert_eq!(names(&witnesses[1]), ["/c", "/d"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_edge_to_a_resource_without_a_vertex_is_rejected() {
        let err = Graph::compile(
            vec![action("/a")],
            vec![edge("/a", "/ghost", EdgeKind::Requires)],
        )
        .unwrap_err();
        assert!(
            matches!(err, CompileError::UnknownResource { missing, .. } if missing == r("/ghost"))
        );
    }

    // Found by mutation calibration (the warp-truth-table record): the key
    // text and the error text could be replaced and no test noticed.
    #[test]
    fn keys_and_errors_say_what_they_are() {
        use alloc::format;
        let key = super::ConflictKey::new("file:/etc/hosts").unwrap();
        assert_eq!(key.as_str(), "file:/etc/hosts");
        assert!(super::ConflictKey::new("").is_none());
        let duplicate = CompileError::DuplicateVertex(r("/a"));
        assert!(format!("{duplicate}").contains("/a"));
        let unknown = CompileError::UnknownResource {
            edge: edge("/a", "/ghost", EdgeKind::Requires),
            missing: r("/ghost"),
        };
        assert!(format!("{unknown}").contains("/ghost"));
        let cycle = CompileError::Cycle {
            witnesses: vec![vec![r("/a")]],
            blocked: vec![],
        };
        assert!(format!("{cycle}").contains("1 cycle"));
    }

    #[test]
    fn two_vertices_for_one_resource_are_rejected() {
        let err = Graph::compile(vec![action("/a"), Vertex::anchor(r("/a"))], vec![]).unwrap_err();
        assert_eq!(err, CompileError::DuplicateVertex(r("/a")));
    }

    #[test]
    fn duplicate_edges_collapse_to_one() {
        let graph = Graph::compile(
            vec![action("/a"), action("/b")],
            vec![
                edge("/a", "/b", EdgeKind::Requires),
                edge("/a", "/b", EdgeKind::Requires),
            ],
        )
        .unwrap();
        assert_eq!(graph.edges().len(), 1);
        assert_eq!(graph.edges_into(&r("/b")).count(), 1);
    }
}
