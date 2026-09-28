//! The execution frontier: which Actions may run now, and why the others
//! may not ([warp.md](../../../../docs/formal/warp.md), ADR 0009).
//!
//! A source is **met** when it Succeeded, changed or not, or was Skipped,
//! and **not met** when it ended any other way: Failed, TimedOut, Cancelled,
//! Rejected, an Indeterminate anchor, or Blocked. `requires` and `on_change`
//! both ask whether their source is met, because both depend on what the
//! source did; `after` asks only whether it is terminal.
//!
//! Readiness is two questions answered separately. **Startable** is edge by
//! edge: every `requires` source met, every `after` and `on_change` source
//! terminal. **Activated** is a property of the `on_change` sources taken
//! together: the vertex has none, or every source is met and at least one
//! succeeded with a verified change. An unchanged source never vetoes a
//! changed one; a source that is not met vetoes the group.
//!
//! Each unit resolves to one state, with no fallthrough:
//!
//! | Unit | States |
//! | --- | --- |
//! | `requires` edge | Waiting, Satisfied, Blocked ([`EdgeState`]) |
//! | `after` edge | Waiting, Satisfied ([`AfterState`]; a separate type, so an `after` edge cannot be Blocked even by mistake) |
//! | `on_change` group | Empty, Blocked, Waiting, Activated, Disabled, checked in that order |
//!
//! A pending vertex is Blocked when a `requires` edge or its group is,
//! Waiting when any unit is, Skipped when its group is Disabled, and Ready
//! otherwise. Blocked and Skipped are final and propagate. A Skipped vertex
//! had no reason to run and every trigger ended well, so it is met: its
//! dependents see it as they see a satisfaction anchor. A Blocked vertex
//! will never run and is not met: it blocks its `requires` and `on_change`
//! dependents and satisfies its `after` dependents. Blocking therefore
//! travels along `requires` and `on_change` edges and stops at `after`
//! edges (ADR 0009 §1 and §2).
//!
//! Anchors have their outcome on entry: a satisfaction anchor Succeeded and
//! unchanged, an Indeterminate anchor Indeterminate. The per-edge effect of
//! an Indeterminate anchor follows from the table: `requires` Blocked,
//! `after` Satisfied, `on_change` Blocked.

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
///
/// There is no Skipped progress. Skipped is met, so a caller able to record
/// it could make dependents run without the frontier checking a single
/// trigger. It is a resolution the frontier derives, afresh on every call,
/// from a pending vertex's sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Progress {
    /// Not started.
    Pending,
    /// Dispatched and not terminal.
    Running,
    /// Terminal with this outcome.
    Done(Outcome),
}

/// What a source vertex looks like to the edges that leave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum SourceState {
    /// Pending or Running, and not yet resolved as final.
    Open,
    /// Terminal with this outcome.
    Done(Outcome),
    /// Resolved Skipped: no reason to run, every trigger met.
    Skipped,
    /// Resolved Blocked: it will never run.
    Blocked,
}

impl SourceState {
    /// Whether the source is terminal, in the lifecycle or by resolution.
    pub fn is_terminal(self) -> bool {
        !matches!(self, SourceState::Open)
    }

    /// Whether the source is met: it Succeeded, changed or not, or was Skipped.
    pub fn is_met(self) -> bool {
        matches!(
            self,
            SourceState::Done(Outcome::Succeeded { .. }) | SourceState::Skipped
        )
    }

    /// Whether the source is terminal and not met: Failed, TimedOut,
    /// Cancelled, Rejected, Indeterminate, or Blocked.
    pub fn is_not_met(self) -> bool {
        self.is_terminal() && !self.is_met()
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
    /// No source is known not met, and some source is not terminal.
    Waiting,
    /// Every source is met, and some source succeeded and changed.
    Activated,
    /// Every source is met and none changed.
    Disabled,
    /// Some source is not met, whether or not the others have finished. Final.
    Blocked,
}

/// The resolution of a pending vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Resolution {
    /// Startable and Activated: it may run now.
    Ready,
    /// Some unit is Waiting.
    Waiting,
    /// A `requires` edge or the `on_change` group is Blocked. Final, not met.
    Blocked,
    /// The `on_change` group is Disabled. Final, met, terminal without change.
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
    } else if sources.iter().any(|s| s.is_not_met()) {
        GroupState::Blocked
    } else if sources.iter().any(|s| !s.is_terminal()) {
        GroupState::Waiting
    } else if sources.iter().any(|s| s.is_changed()) {
        GroupState::Activated
    } else {
        GroupState::Disabled
    }
}

/// The resolution of a pending vertex from its units. An `owed` vertex has
/// a pending Obligation, which is its reason to run, so a Disabled group
/// makes it Ready instead of Skipped (ADR 0009 note).
pub fn resolve_vertex(
    requires: &[EdgeState],
    after: &[AfterState],
    group: GroupState,
    owed: bool,
) -> Resolution {
    if requires.contains(&EdgeState::Blocked) || group == GroupState::Blocked {
        Resolution::Blocked
    } else if requires.contains(&EdgeState::Waiting)
        || after.contains(&AfterState::Waiting)
        || group == GroupState::Waiting
    {
        Resolution::Waiting
    } else if group == GroupState::Disabled && !owed {
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
                let resolution = resolve_vertex(
                    &requires,
                    &after,
                    resolve_group(&group_sources),
                    vertex.is_owed(),
                );
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
        use GroupState::{Activated, Blocked, Disabled, Waiting};
        assert_eq!(resolve_group(&[]), GroupState::Empty);
        // One row per source state, written out rather than derived.
        let table: [(SourceState, GroupState); 10] = [
            (SourceState::Open, Waiting),
            (
                SourceState::Done(Outcome::Succeeded { changed: true }),
                Activated,
            ),
            (
                SourceState::Done(Outcome::Succeeded { changed: false }),
                Disabled,
            ),
            (SourceState::Done(Outcome::Failed), Blocked),
            (SourceState::Done(Outcome::TimedOut), Blocked),
            (SourceState::Done(Outcome::Cancelled), Blocked),
            (SourceState::Done(Outcome::Rejected), Blocked),
            (SourceState::Done(Outcome::Indeterminate), Blocked),
            (SourceState::Skipped, Disabled),
            (SourceState::Blocked, Blocked),
        ];
        assert_eq!(table.len(), SOURCES.len());
        for (source, expected) in table {
            assert_eq!(resolve_group(&[source]), expected, "{source:?}");
        }
        let changed = SourceState::Done(Outcome::Succeeded { changed: true });
        let unchanged = SourceState::Done(Outcome::Succeeded { changed: false });
        let failed = SourceState::Done(Outcome::Failed);
        // An unchanged source never vetoes a changed one.
        assert_eq!(resolve_group(&[changed, unchanged]), Activated);
        assert_eq!(resolve_group(&[unchanged, changed]), Activated);
        assert_eq!(resolve_group(&[changed, SourceState::Open]), Waiting);
        assert_eq!(resolve_group(&[unchanged, SourceState::Skipped]), Disabled);
        // A source that is not met vetoes the group, changed sibling or not,
        // and blocks it before the other sources finish.
        assert_eq!(resolve_group(&[changed, failed]), Blocked);
        assert_eq!(resolve_group(&[failed, changed]), Blocked);
        assert_eq!(resolve_group(&[changed, SourceState::Blocked]), Blocked);
        assert_eq!(
            resolve_group(&[unchanged, SourceState::Done(Outcome::Indeterminate)]),
            Blocked
        );
        assert_eq!(resolve_group(&[SourceState::Open, failed]), Blocked);
    }

    #[test]
    fn a_vertex_resolves_to_exactly_one_state() {
        use EdgeState::{Blocked, Satisfied, Waiting};
        let after_met = AfterState::Satisfied;
        assert_eq!(
            resolve_vertex(&[], &[], GroupState::Empty, false),
            Resolution::Ready
        );
        assert_eq!(
            resolve_vertex(&[Satisfied], &[after_met], GroupState::Activated, false),
            Resolution::Ready
        );
        assert_eq!(
            resolve_vertex(&[Blocked, Waiting], &[], GroupState::Waiting, false),
            Resolution::Blocked
        );
        assert_eq!(
            resolve_vertex(&[Waiting], &[], GroupState::Disabled, false),
            Resolution::Waiting
        );
        assert_eq!(
            resolve_vertex(&[], &[AfterState::Waiting], GroupState::Activated, false),
            Resolution::Waiting
        );
        assert_eq!(
            resolve_vertex(&[Satisfied], &[], GroupState::Waiting, false),
            Resolution::Waiting
        );
        assert_eq!(
            resolve_vertex(&[Satisfied], &[after_met], GroupState::Disabled, false),
            Resolution::Skipped
        );
        assert_eq!(
            resolve_vertex(&[Satisfied], &[after_met], GroupState::Blocked, false),
            Resolution::Blocked
        );
        assert_eq!(
            resolve_vertex(
                &[Waiting],
                &[AfterState::Waiting],
                GroupState::Blocked,
                false
            ),
            Resolution::Blocked
        );
    }

    /// The owed rows of the vertex table (ADR 0009 note): an Obligation
    /// replaces activation as the reason to run, and nothing else changes.
    #[test]
    fn an_owed_vertex_differs_only_where_its_group_is_disabled() {
        use EdgeState::{Blocked, Satisfied, Waiting};
        let requires_rows: [&[EdgeState]; 4] = [&[], &[Satisfied], &[Waiting], &[Blocked]];
        let after_rows: [&[AfterState]; 3] =
            [&[], &[AfterState::Satisfied], &[AfterState::Waiting]];
        let groups = [
            GroupState::Empty,
            GroupState::Waiting,
            GroupState::Activated,
            GroupState::Disabled,
            GroupState::Blocked,
        ];
        for requires in requires_rows {
            for after in after_rows {
                for group in groups {
                    let plain = resolve_vertex(requires, after, group, false);
                    let owed = resolve_vertex(requires, after, group, true);
                    let expected = if plain == Resolution::Skipped {
                        Resolution::Ready
                    } else {
                        plain
                    };
                    assert_eq!(owed, expected, "{requires:?} {after:?} {group:?}");
                }
            }
        }
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

    fn owed(text: &str) -> Vertex {
        Vertex::owed(r(text), BTreeSet::new())
    }

    /// Semantic mutant `SM-WARP-006`. The run after a crash sees the
    /// configuration Satisfied, so its trigger is an unchanged anchor and
    /// the group is Disabled; the pending Obligation makes the refresh Ready,
    /// and its `requires` dependent waits for it rather than being Skipped
    /// past it.
    #[test]
    fn an_owed_refresh_runs_when_its_group_is_disabled() {
        let f = run(
            vec![Vertex::anchor(r("/conf")), owed("/svc"), action("/check")],
            vec![
                edge("/conf", "/svc", EdgeKind::OnChange),
                edge("/svc", "/check", EdgeKind::Requires),
            ],
            &[],
        );
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Ready));
        assert_eq!(f.resolution(&r("/check")), Some(Resolution::Waiting));
        assert_eq!(f.ready(), [r("/svc")]);
        // The same refresh without the Obligation had no reason to run.
        let f = run(
            vec![Vertex::anchor(r("/conf")), action("/svc")],
            vec![edge("/conf", "/svc", EdgeKind::OnChange)],
            &[],
        );
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Skipped));
    }

    /// An owed refresh still waits for its trigger and is still Blocked by
    /// one that is not met: the Obligation is a reason to run, not
    /// permission to load a failed input.
    #[test]
    fn an_owed_refresh_waits_for_and_is_blocked_by_its_trigger() {
        let vertices = || vec![action("/conf"), owed("/svc")];
        let edges = || vec![edge("/conf", "/svc", EdgeKind::OnChange)];
        let f = run(vertices(), edges(), &[]);
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Waiting));
        let f = run(vertices(), edges(), &[("/conf", Progress::Running)]);
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Waiting));
        for outcome in [
            Outcome::Failed,
            Outcome::TimedOut,
            Outcome::Cancelled,
            Outcome::Rejected,
        ] {
            let f = run(vertices(), edges(), &[("/conf", Progress::Done(outcome))]);
            assert_eq!(
                f.resolution(&r("/svc")),
                Some(Resolution::Blocked),
                "{outcome:?}"
            );
        }
        let f = run(
            vec![Vertex::indeterminate_anchor(r("/conf")), owed("/svc")],
            edges(),
            &[],
        );
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Blocked));
    }

    /// The run that repairs the failed trigger: the write succeeds, whether
    /// or not it changes the file this time, and the refresh the Obligation
    /// still owes is Ready.
    #[test]
    fn a_blocked_refresh_runs_once_its_trigger_is_repaired() {
        let blocked = run(
            vec![action("/conf"), action("/svc")],
            vec![edge("/conf", "/svc", EdgeKind::OnChange)],
            &[("/conf", Progress::Done(Outcome::Failed))],
        );
        assert_eq!(blocked.resolution(&r("/svc")), Some(Resolution::Blocked));
        for changed in [true, false] {
            let repaired = run(
                vec![action("/conf"), owed("/svc")],
                vec![edge("/conf", "/svc", EdgeKind::OnChange)],
                &[("/conf", Progress::Done(Outcome::Succeeded { changed }))],
            );
            assert_eq!(
                repaired.resolution(&r("/svc")),
                Some(Resolution::Ready),
                "changed: {changed}"
            );
        }
    }

    #[test]
    fn only_an_action_disrupts() {
        let nodes: BTreeSet<crate::budget::Node> = [crate::budget::Node::new("n").unwrap()]
            .into_iter()
            .collect();
        assert_eq!(action("/a").disrupting(nodes.clone()).disrupts(), &nodes);
        assert!(
            Vertex::anchor(r("/a"))
                .disrupting(nodes.clone())
                .disrupts()
                .is_empty()
        );
        assert!(
            Vertex::indeterminate_anchor(r("/a"))
                .disrupting(nodes)
                .disrupts()
                .is_empty()
        );
        assert!(owed("/a").is_owed());
        assert!(!action("/a").is_owed());
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

    /// A failed or unknown trigger is not "no change": the refresh is
    /// Blocked, not Skipped, so nothing that requires the refresh runs as if
    /// it had been unnecessary. Blocking propagates along `requires` and
    /// `on_change` and stops at `after` (semantic mutant `SM-WARP-004`).
    #[test]
    fn a_failed_trigger_blocks_the_refresh_and_what_requires_it() {
        for outcome in [
            Outcome::Failed,
            Outcome::TimedOut,
            Outcome::Cancelled,
            Outcome::Rejected,
        ] {
            let f = run(
                vec![
                    action("/conf"),
                    action("/svc"),
                    action("/check"),
                    action("/reload"),
                    action("/log"),
                ],
                vec![
                    edge("/conf", "/svc", EdgeKind::OnChange),
                    edge("/svc", "/check", EdgeKind::Requires),
                    edge("/svc", "/reload", EdgeKind::OnChange),
                    edge("/svc", "/log", EdgeKind::After),
                ],
                &[("/conf", Progress::Done(outcome))],
            );
            assert_eq!(
                f.resolution(&r("/svc")),
                Some(Resolution::Blocked),
                "{outcome:?}"
            );
            assert_eq!(
                f.resolution(&r("/check")),
                Some(Resolution::Blocked),
                "{outcome:?}"
            );
            assert_eq!(
                f.resolution(&r("/reload")),
                Some(Resolution::Blocked),
                "{outcome:?}"
            );
            assert_eq!(
                f.resolution(&r("/log")),
                Some(Resolution::Ready),
                "{outcome:?}"
            );
        }
    }

    /// A changed source does not activate a refresh beside a failed one:
    /// the refresh would load an input whose write failed, or one that a
    /// timed-out write may still be changing (semantic mutant `SM-WARP-005`).
    #[test]
    fn a_changed_source_beside_a_failed_one_does_not_activate() {
        for outcome in [Outcome::Failed, Outcome::TimedOut] {
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
                    ("/conf-b", Progress::Done(outcome)),
                ],
            );
            assert_eq!(
                f.resolution(&r("/svc")),
                Some(Resolution::Blocked),
                "{outcome:?}"
            );
            assert!(f.ready().is_empty());
        }
    }

    /// One failed trigger blocks the refresh before the other triggers
    /// finish, as one failed prerequisite blocks a `requires` dependent.
    #[test]
    fn a_failed_trigger_blocks_while_another_is_running() {
        let f = run(
            vec![action("/conf-a"), action("/conf-b"), action("/svc")],
            vec![
                edge("/conf-a", "/svc", EdgeKind::OnChange),
                edge("/conf-b", "/svc", EdgeKind::OnChange),
            ],
            &[
                ("/conf-a", Progress::Running),
                ("/conf-b", Progress::Done(Outcome::Failed)),
            ],
        );
        assert_eq!(f.resolution(&r("/svc")), Some(Resolution::Blocked));
    }

    /// The Indeterminate-anchor rows of ADR 0009 §3: `requires` and
    /// `on_change` Blocked, `after` Satisfied.
    #[test]
    fn an_indeterminate_anchor_blocks_requires_and_on_change_and_satisfies_after() {
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
        assert_eq!(f.resolution(&r("/chg")), Some(Resolution::Blocked));
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

    /// A Skipped source had no reason to run and every trigger ended well:
    /// met for `requires`, unchanged for `on_change` (ADR 0009 §2).
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
