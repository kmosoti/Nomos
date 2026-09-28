//! The execution frontier: which Actions may run now, and why the others
//! may not ([warp.md](../../../../docs/formal/warp.md), ADR 0009).
//!
//! Readiness is two questions answered separately. **Startable** is edge by
//! edge: every `requires` source Succeeded, every `after` and `on_change`
//! source terminal. **Activated** is a property of the `on_change` sources
//! taken together: the vertex has none, or at least one succeeded with a
//! verified change. A single unchanged source never vetoes a changed one.
//!
//! Each unit resolves to one state, with no fallthrough:
//!
//! | Unit | States |
//! | --- | --- |
//! | `requires` edge | Waiting, Satisfied, Blocked ([`EdgeState`]) |
//! | `after` edge | Waiting, Satisfied ([`AfterState`]; a separate type, so an `after` edge cannot be Blocked even by mistake) |
//! | `on_change` group | Empty, Waiting, Activated, Disabled |
//!
//! A pending vertex is Blocked when any `requires` edge is, Waiting when any
//! unit is, Skipped when its group is Disabled, and Ready otherwise. Blocked
//! and Skipped are final resolutions and propagate: a Blocked source blocks
//! its `requires` dependents and is terminal for `after`; a Skipped source
//! is terminal without change, so it counts as met for `requires` and
//! `after` and toward Disabled for `on_change`. That reading of Skipped,
//! "not failed, terminal without change", is a working definition this
//! milestone records for ADR 0009; the alternative, treating Skipped as
//! ended other than Succeeded, would block every `requires` dependent of a
//! refresh that had no reason to run.
//!
//! Anchors have their outcome on entry: a satisfaction anchor Succeeded and
//! unchanged, an Indeterminate anchor Indeterminate. The per-edge effect of
//! an Indeterminate anchor follows from the table: `requires` Blocked,
//! `after` Satisfied, `on_change` a source without a change.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use nomos_core::resource::ResourcePath;

use crate::graph::{EdgeKind, Graph, VertexKind};

/// The terminal outcome of an Action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Outcome {
    /// Verified postconditions (N6), with whether verification observed a change.
    Succeeded {
        /// Whether verification observed a state change.
        changed: bool,
    },
    /// The operation failed.
    Failed,
    /// The outcome is unknown and stays unknown (N10).
    TimedOut,
    /// Cancelled before completion.
    Cancelled,
    /// Rejected before dispatch, for example by fencing (N5).
    Rejected,
    /// An Indeterminate anchor's outcome: nothing ran and nothing is known.
    Indeterminate,
}

/// Where an Action is in its lifecycle, as the frontier sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Progress {
    /// Not started.
    Pending,
    /// Dispatched and not terminal.
    Running,
    /// Terminal with this outcome.
    Done(Outcome),
    /// Terminal without change: it had no reason to run.
    Skipped,
}

/// What a source vertex looks like to the edges that leave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum SourceState {
    /// Pending or Running, and not yet resolved as final.
    Open,
    /// Terminal with this outcome.
    Done(Outcome),
    /// Terminal without change, by progress or by resolution.
    Skipped,
    /// Resolved Blocked: it will never run.
    Blocked,
}

impl SourceState {
    /// Whether the source is terminal, in the lifecycle or by resolution.
    pub fn is_terminal(self) -> bool {
        !matches!(self, SourceState::Open)
    }

    /// Whether the source counts as a met prerequisite.
    pub fn is_met(self) -> bool {
        matches!(
            self,
            SourceState::Done(Outcome::Succeeded { .. }) | SourceState::Skipped
        )
    }

    /// Whether the source succeeded with a verified change.
    pub fn is_changed(self) -> bool {
        matches!(
            self,
            SourceState::Done(Outcome::Succeeded { changed: true })
        )
    }
}

/// The state of one `requires` edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum EdgeState {
    /// The source is not terminal.
    Waiting,
    /// The edge is met.
    Satisfied,
    /// The source ended other than met. Propagates.
    Blocked,
}

/// The state of one `after` edge. `after` is ordering, not success, so it
/// has no Blocked state; the type says so, and a Kani harness on
/// `resolve_vertex` is what showed that a shared type let one in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum AfterState {
    /// The source is not terminal.
    Waiting,
    /// The source is terminal, whatever its outcome.
    Satisfied,
}

/// The state of a vertex's `on_change` sources, as a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum GroupState {
    /// No `on_change` source: activation is not in question.
    Empty,
    /// Some source is not terminal.
    Waiting,
    /// Some source succeeded and changed.
    Activated,
    /// Every source is terminal and none changed.
    Disabled,
}

/// The resolution of a pending vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Resolution {
    /// Startable and Activated: it may run now.
    Ready,
    /// Some unit is Waiting.
    Waiting,
    /// A `requires` edge is Blocked. Final.
    Blocked,
    /// The `on_change` group is Disabled. Final, terminal without change.
    Skipped,
}

/// A `requires` edge from a source in `state`.
pub fn resolve_requires(state: SourceState) -> EdgeState {
    if !state.is_terminal() {
        EdgeState::Waiting
    } else if state.is_met() {
        EdgeState::Satisfied
    } else {
        EdgeState::Blocked
    }
}

/// An `after` edge from a source in `state`.
pub fn resolve_after(state: SourceState) -> AfterState {
    if state.is_terminal() {
        AfterState::Satisfied
    } else {
        AfterState::Waiting
    }
}

/// The `on_change` sources of one vertex, as a group.
pub fn resolve_group(sources: &[SourceState]) -> GroupState {
    if sources.is_empty() {
        GroupState::Empty
    } else if sources.iter().any(|s| !s.is_terminal()) {
        GroupState::Waiting
    } else if sources.iter().any(|s| s.is_changed()) {
        GroupState::Activated
    } else {
        GroupState::Disabled
    }
}

/// The resolution of a pending vertex from its units.
pub fn resolve_vertex(
    requires: &[EdgeState],
    after: &[AfterState],
    group: GroupState,
) -> Resolution {
    if requires.contains(&EdgeState::Blocked) {
        Resolution::Blocked
    } else if requires.contains(&EdgeState::Waiting)
        || after.contains(&AfterState::Waiting)
        || group == GroupState::Waiting
    {
        Resolution::Waiting
    } else if group == GroupState::Disabled {
        Resolution::Skipped
    } else {
        Resolution::Ready
    }
}

/// The frontier over a graph: the resolution of every pending vertex, and
/// the Ready ones in topological order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontier {
    resolutions: BTreeMap<ResourcePath, Resolution>,
    ready: Vec<ResourcePath>,
}

impl Frontier {
    /// The resolution of a pending vertex; `None` for a vertex that is not pending.
    pub fn resolution(&self, resource: &ResourcePath) -> Option<Resolution> {
        self.resolutions.get(resource).copied()
    }

    /// Every pending vertex with its resolution, by resource.
    pub fn resolutions(&self) -> &BTreeMap<ResourcePath, Resolution> {
        &self.resolutions
    }

    /// The Ready vertices, in the graph's topological order.
    pub fn ready(&self) -> &[ResourcePath] {
        &self.ready
    }
}

/// The frontier of `graph` given the progress of its Actions. An Action
/// absent from `progress` is Pending; anchors have their outcome on entry.
pub fn frontier(graph: &Graph, progress: &BTreeMap<ResourcePath, Progress>) -> Frontier {
    let mut states: BTreeMap<&ResourcePath, SourceState> = BTreeMap::new();
    let mut resolutions = BTreeMap::new();
    let mut ready = Vec::new();
    // Topological order: every source is classified before its dependents.
    for resource in graph.order() {
        let Some(vertex) = graph.vertex(resource) else {
            continue;
        };
        let effective = match vertex.kind() {
            VertexKind::Anchor => Progress::Done(Outcome::Succeeded { changed: false }),
            VertexKind::IndeterminateAnchor => Progress::Done(Outcome::Indeterminate),
            VertexKind::Action => progress.get(resource).copied().unwrap_or(Progress::Pending),
        };
        let state = match effective {
            Progress::Pending => {
                let source = |edge_source: &ResourcePath| {
                    states
                        .get(edge_source)
                        .copied()
                        .unwrap_or(SourceState::Open)
                };
                let requires: Vec<EdgeState> = graph
                    .edges_into(resource)
                    .filter(|e| e.kind() == EdgeKind::Requires)
                    .map(|e| resolve_requires(source(e.source())))
                    .collect();
                let after: Vec<AfterState> = graph
                    .edges_into(resource)
                    .filter(|e| e.kind() == EdgeKind::After)
                    .map(|e| resolve_after(source(e.source())))
                    .collect();
                let group_sources: Vec<SourceState> = graph
                    .edges_into(resource)
                    .filter(|e| e.kind() == EdgeKind::OnChange)
                    .map(|e| source(e.source()))
                    .collect();
                let resolution = resolve_vertex(&requires, &after, resolve_group(&group_sources));
                resolutions.insert(resource.clone(), resolution);
                match resolution {
                    Resolution::Ready => {
                        ready.push(resource.clone());
                        SourceState::Open
                    }
                    Resolution::Waiting => SourceState::Open,
                    Resolution::Blocked => SourceState::Blocked,
                    Resolution::Skipped => SourceState::Skipped,
                }
            }
            Progress::Running => SourceState::Open,
            Progress::Done(outcome) => SourceState::Done(outcome),
            Progress::Skipped => SourceState::Skipped,
        };
        states.insert(resource, state);
    }
    Frontier { resolutions, ready }
}

#[cfg(test)]
mod tests {
    use alloc::collections::{BTreeMap, BTreeSet};
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{
        AfterState, EdgeState, GroupState, Outcome, Progress, Resolution, SourceState, frontier,
        resolve_after, resolve_group, resolve_requires, resolve_vertex,
    };
    use crate::graph::{Edge, EdgeKind, Graph, Vertex};
    use nomos_core::resource::ResourcePath;

    fn r(text: &str) -> ResourcePath {
        ResourcePath::new(text).unwrap()
    }

    fn action(text: &str) -> Vertex {
        Vertex::action(r(text), BTreeSet::new())
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge::new(r(from), r(to), kind)
    }

    /// Every source state the table has a row for.
    const SOURCES: [SourceState; 10] = [
        SourceState::Open,
        SourceState::Done(Outcome::Succeeded { changed: true }),
        SourceState::Done(Outcome::Succeeded { changed: false }),
        SourceState::Done(Outcome::Failed),
        SourceState::Done(Outcome::TimedOut),
        SourceState::Done(Outcome::Cancelled),
        SourceState::Done(Outcome::Rejected),
        SourceState::Done(Outcome::Indeterminate),
        SourceState::Skipped,
        SourceState::Blocked,
    ];

    /// The exhaustive predecessor-outcome table for `requires` and `after`
    /// edges, every source outcome and the Indeterminate anchor included
    /// (ADR 0009 acceptance criterion).
    #[test]
    fn the_edge_truth_table_is_exhaustive() {
        use EdgeState::{Blocked, Satisfied, Waiting};
        let met = AfterState::Satisfied;
        let table: [(SourceState, EdgeState, AfterState); 10] = [
            (SourceState::Open, Waiting, AfterState::Waiting),
            (
                SourceState::Done(Outcome::Succeeded { changed: true }),
                Satisfied,
                met,
            ),
            (
                SourceState::Done(Outcome::Succeeded { changed: false }),
                Satisfied,
                met,
            ),
            (SourceState::Done(Outcome::Failed), Blocked, met),
            (SourceState::Done(Outcome::TimedOut), Blocked, met),
            (SourceState::Done(Outcome::Cancelled), Blocked, met),
            (SourceState::Done(Outcome::Rejected), Blocked, met),
            (SourceState::Done(Outcome::Indeterminate), Blocked, met),
            (SourceState::Skipped, Satisfied, met),
            (SourceState::Blocked, Blocked, met),
        ];
        assert_eq!(table.len(), SOURCES.len());
        for (source, requires, after) in table {
            assert_eq!(
                resolve_requires(source),
                requires,
                "requires from {source:?}"
            );
            assert_eq!(resolve_after(source), after, "after from {source:?}");
        }
    }

    /// The `on_change` group table over every single source, then the
    /// mixed cases the definition exists for.
    #[test]
    fn the_group_truth_table_is_exhaustive() {
        assert_eq!(resolve_group(&[]), GroupState::Empty);
        for source in SOURCES {
            let expected = if !source.is_terminal() {
                GroupState::Waiting
            } else if source.is_changed() {
                GroupState::Activated
            } else {
                GroupState::Disabled
            };
            assert_eq!(resolve_group(&[source]), expected, "{source:?}");
        }
        let changed = SourceState::Done(Outcome::Succeeded { changed: true });
        let unchanged = SourceState::Done(Outcome::Succeeded { changed: false });
        assert_eq!(resolve_group(&[changed, unchanged]), GroupState::Activated);
        assert_eq!(resolve_group(&[unchanged, changed]), GroupState::Activated);
        assert_eq!(
            resolve_group(&[changed, SourceState::Open]),
            GroupState::Waiting
        );
        assert_eq!(
            resolve_group(&[unchanged, SourceState::Done(Outcome::Indeterminate)]),
            GroupState::Disabled
        );
        assert_eq!(
            resolve_group(&[unchanged, SourceState::Skipped]),
            GroupState::Disabled
        );
        assert_eq!(
            resolve_group(&[changed, SourceState::Blocked]),
            GroupState::Activated
        );
    }

    #[test]
    fn a_vertex_resolves_to_exactly_one_state() {
        use EdgeState::{Blocked, Satisfied, Waiting};
        let after_met = AfterState::Satisfied;
        assert_eq!(
            resolve_vertex(&[], &[], GroupState::Empty),
            Resolution::Ready
        );
        assert_eq!(
            resolve_vertex(&[Satisfied], &[after_met], GroupState::Activated),
            Resolution::Ready
        );
        assert_eq!(
            resolve_vertex(&[Blocked, Waiting], &[], GroupState::Waiting),
            Resolution::Blocked
        );
        assert_eq!(
            resolve_vertex(&[Waiting], &[], GroupState::Disabled),
            Resolution::Waiting
        );
        assert_eq!(
            resolve_vertex(&[], &[AfterState::Waiting], GroupState::Activated),
            Resolution::Waiting
        );
        assert_eq!(
            resolve_vertex(&[Satisfied], &[], GroupState::Waiting),
            Resolution::Waiting
        );
        assert_eq!(
            resolve_vertex(&[Satisfied], &[after_met], GroupState::Disabled),
            Resolution::Skipped
        );
    }

    fn run(
        vertices: Vec<Vertex>,
        edges: Vec<Edge>,
        progress: &[(&str, Progress)],
    ) -> super::Frontier {
        let graph = Graph::compile(vertices, edges).unwrap();
        let progress: BTreeMap<ResourcePath, Progress> =
            progress.iter().map(|(p, s)| (r(p), *s)).collect();
        frontier(&graph, &progress)
    }

    /// N4 (semantic mutant `SM-WARP-001`).
    #[test]
    fn a_failed_requirement_blocks_the_dependent() {
        for outcome in [
            Outcome::Failed,
            Outcome::TimedOut,
            Outcome::Cancelled,
            Outcome::Rejected,
        ] {
            let f = run(
                vec![action("/a"), action("/b"), action("/c")],
                vec![
                    edge("/a", "/b", EdgeKind::Requires),
                    edge("/b", "/c", EdgeKind::Requires),
                ],
                &[("/a", Progress::Done(outcome))],
            );
            assert_eq!(
                f.resolution(&r("/b")),
                Some(Resolution::Blocked),
                "{outcome:?}"
            );
            assert_eq!(
                f.resolution(&r("/c")),
                Some(Resolution::Blocked),
                "propagates"
            );
            assert!(f.ready().is_empty());
        }
    }

    /// `after` is ordering, not success (semantic mutant `SM-WARP-002`).
    #[test]
    fn a_failed_after_edge_does_not_block() {
        let f = run(
            vec![action("/a"), action("/b")],
            vec![edge("/a", "/b", EdgeKind::After)],
            &[("/a", Progress::Done(Outcome::Failed))],
        );
        assert_eq!(f.resolution(&r("/b")), Some(Resolution::Ready));
        assert_eq!(f.ready(), [r("/b")]);
    }

    #[test]
    fn a_running_source_keeps_its_dependents_waiting() {
        let f = run(
            vec![action("/a"), action("/b"), action("/c")],
            vec![
                edge("/a", "/b", EdgeKind::Requires),
                edge("/a", "/c", EdgeKind::After),
            ],
            &[("/a", Progress::Running)],
        );
        assert_eq!(f.resolution(&r("/b")), Some(Resolution::Waiting));
        assert_eq!(f.resolution(&r("/c")), Some(Resolution::Waiting));
        assert_eq!(
            f.resolution(&r("/a")),
            None,
            "a Running vertex is not pending"
        );
    }

    /// Counterexample `activation-missing`: one changed source beside one
    /// unchanged source activates the group.
    #[test]
    fn one_changed_source_activates_beside_an_unchanged_one() {
        let f = run(
            vec![action("/conf-a"), action("/conf-b"), action("/svc")],
            vec![
                edge("/conf-a", "/svc", EdgeKind::OnChange),
                edge("/conf-b", "/svc", EdgeKind::OnChange),
            ],
            &[
                (
                    "/conf-a",
                    Progress::Done(Outcome::Succeeded { changed: true }),
                ),
                (
                    "/conf-b",
                    Progress::Done(Outcome::Succeeded { changed: false }),
                ),
            ],
        );
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Ready));
    }

    #[test]
    fn an_unchanged_configuration_does_not_activate_a_refresh() {
        let f = run(
            vec![action("/conf"), action("/svc")],
            vec![edge("/conf", "/svc", EdgeKind::OnChange)],
            &[(
                "/conf",
                Progress::Done(Outcome::Succeeded { changed: false }),
            )],
        );
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Skipped));
        assert!(f.ready().is_empty());
    }

    /// A failed partial write is terminal and did not produce the change the
    /// edge waits for.
    #[test]
    fn a_failed_source_does_not_activate_its_dependent() {
        let f = run(
            vec![action("/conf"), action("/svc")],
            vec![edge("/conf", "/svc", EdgeKind::OnChange)],
            &[("/conf", Progress::Done(Outcome::Failed))],
        );
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Skipped));
    }

    /// The Indeterminate-anchor rows of ADR 0009 §3.
    #[test]
    fn an_indeterminate_anchor_blocks_requires_satisfies_after_and_never_activates() {
        let f = run(
            vec![
                Vertex::indeterminate_anchor(r("/x")),
                action("/req"),
                action("/aft"),
                action("/chg"),
            ],
            vec![
                edge("/x", "/req", EdgeKind::Requires),
                edge("/x", "/aft", EdgeKind::After),
                edge("/x", "/chg", EdgeKind::OnChange),
            ],
            &[],
        );
        assert_eq!(f.resolution(&r("/req")), Some(Resolution::Blocked));
        assert_eq!(f.resolution(&r("/aft")), Some(Resolution::Ready));
        assert_eq!(f.resolution(&r("/chg")), Some(Resolution::Skipped));
    }

    /// A satisfaction anchor is Succeeded and unchanged on entry.
    #[test]
    fn a_satisfaction_anchor_meets_requires_and_after_and_never_activates() {
        let f = run(
            vec![
                Vertex::anchor(r("/x")),
                action("/req"),
                action("/aft"),
                action("/chg"),
            ],
            vec![
                edge("/x", "/req", EdgeKind::Requires),
                edge("/x", "/aft", EdgeKind::After),
                edge("/x", "/chg", EdgeKind::OnChange),
            ],
            &[],
        );
        assert_eq!(f.resolution(&r("/req")), Some(Resolution::Ready));
        assert_eq!(f.resolution(&r("/aft")), Some(Resolution::Ready));
        assert_eq!(f.resolution(&r("/chg")), Some(Resolution::Skipped));
        assert_eq!(f.ready(), [r("/aft"), r("/req")]);
    }

    /// Working definition: a Skipped source is terminal without change.
    #[test]
    fn a_skipped_source_is_met_for_requires_and_unchanged_for_on_change() {
        let f = run(
            vec![
                action("/conf"),
                action("/svc"),
                action("/req"),
                action("/chg"),
            ],
            vec![
                edge("/conf", "/svc", EdgeKind::OnChange),
                edge("/svc", "/req", EdgeKind::Requires),
                edge("/svc", "/chg", EdgeKind::OnChange),
            ],
            &[(
                "/conf",
                Progress::Done(Outcome::Succeeded { changed: false }),
            )],
        );
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Skipped));
        assert_eq!(f.resolution(&r("/req")), Some(Resolution::Ready));
        assert_eq!(f.resolution(&r("/chg")), Some(Resolution::Skipped));
    }

    #[test]
    fn ready_vertices_come_out_in_topological_order() {
        let f = run(
            vec![action("/c"), action("/a"), action("/b")],
            vec![edge("/c", "/a", EdgeKind::After)],
            &[("/c", Progress::Done(Outcome::Succeeded { changed: true }))],
        );
        // Kahn's ordered queue pops /b before /c, so /b precedes /a.
        assert_eq!(f.ready(), [r("/b"), r("/a")]);
    }
}
