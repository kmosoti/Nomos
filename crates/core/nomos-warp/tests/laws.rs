//! Laws of the Warp kernel on generated graphs, checked against a
//! reference written by another route.
//!
//! The reference evaluator here is deliberately naive: cycle detection by
//! depth-first coloring rather than Kahn's algorithm, and the frontier by
//! repeated whole-graph passes until nothing changes rather than a single
//! topological sweep. Disagreement between it and the production functions
//! is a finding about one of them and is never resolved by editing the
//! reference to agree (ADR 0015 §6). The generators produce graphs of up to
//! six resources with up to eight edges of every kind, cycles included, and
//! progress for every Action, owed vertices for pending Obligations, and
//! disruption over four nodes with up to two failure-domain budgets; they do
//! not produce graphs larger than that, which the record states.
//!
//! The runner is seeded from a fixed seed bank, as in `nomos-core`.

use std::collections::{BTreeMap, BTreeSet};

use nomos_core::resource::{ResourceKey, ResourcePath};
use nomos_warp::budget::{Budget, Budgets, Node};
use nomos_warp::frontier::{Outcome, Progress, Resolution, frontier};
use nomos_warp::graph::{CompileError, ConflictKey, Edge, EdgeKind, Graph, Vertex, VertexKind};
use nomos_warp::select::{Reserved, select};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

const SEEDS: [[u8; 32]; 4] = [
    [
        0x4e, 0x34, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0,
    ],
    [
        0x4e, 0x34, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0,
    ],
    [
        0x4e, 0x34, 0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0,
    ],
    [
        0x4e, 0x34, 0x04, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0,
    ],
];

const CASES: u32 = 512;
const RESOURCES: usize = 6;

fn runner(seed: [u8; 32]) -> TestRunner {
    let config = Config {
        cases: CASES,
        failure_persistence: None,
        ..Config::default()
    };
    TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &seed))
}

fn path(n: usize) -> ResourceKey {
    ResourceKey::File(ResourcePath::new(&format!("/r/{n}")).unwrap())
}

fn key(n: u8) -> ConflictKey {
    ConflictKey::new(&format!("k{n}")).unwrap()
}

fn node(n: u8) -> Node {
    Node::new(&format!("n{n}")).unwrap()
}

fn nodes() -> impl Strategy<Value = BTreeSet<Node>> {
    prop::collection::btree_set(0u8..4, 0..3).prop_map(|ns| ns.into_iter().map(node).collect())
}

/// A vertex's kind, keys, its reason of its own to run (0 none, 1 an owed
/// Obligation, 2 its own Variance), and the nodes it disrupts.
type RawVertex = (VertexKind, BTreeSet<ConflictKey>, u8, BTreeSet<Node>);

fn vertex() -> impl Strategy<Value = RawVertex> {
    (
        prop_oneof![
            4 => Just(VertexKind::Action),
            1 => Just(VertexKind::Anchor),
            1 => Just(VertexKind::IndeterminateAnchor),
        ],
        prop::collection::btree_set(0u8..3, 0..3).prop_map(|ks| ks.into_iter().map(key).collect()),
        prop_oneof![4 => Just(0u8), 1 => Just(1u8), 2 => Just(2u8)],
        nodes(),
    )
}

fn kind() -> impl Strategy<Value = EdgeKind> {
    prop_oneof![
        Just(EdgeKind::Requires),
        Just(EdgeKind::After),
        Just(EdgeKind::OnChange)
    ]
}

fn outcome() -> impl Strategy<Value = Outcome> {
    prop_oneof![
        Just(Outcome::Succeeded { changed: true }),
        Just(Outcome::Succeeded { changed: false }),
        Just(Outcome::Failed),
        Just(Outcome::TimedOut),
        Just(Outcome::Cancelled),
        Just(Outcome::Rejected),
    ]
}

fn progress() -> impl Strategy<Value = Progress> {
    prop_oneof![
        3 => Just(Progress::Pending),
        1 => Just(Progress::Running),
        3 => outcome().prop_map(Progress::Done),
    ]
}

/// A graph as lists, possibly cyclic, plus progress for every resource.
type Raw = (Vec<RawVertex>, Vec<(usize, usize, EdgeKind)>, Vec<Progress>);

fn raw() -> impl Strategy<Value = Raw> {
    (
        prop::collection::vec(vertex(), RESOURCES),
        prop::collection::vec((0..RESOURCES, 0..RESOURCES, kind()), 0..9),
        prop::collection::vec(progress(), RESOURCES),
    )
}

fn build(raw: &Raw) -> (Vec<Vertex>, Vec<Edge>, BTreeMap<ResourceKey, Progress>) {
    let vertices = raw
        .0
        .iter()
        .enumerate()
        .map(|(i, (kind, keys, owed, disrupts))| {
            match (kind, owed) {
                (VertexKind::Action, 1) => Vertex::owed(path(i), keys.clone()),
                (VertexKind::Action, 2) => Vertex::varying(path(i), keys.clone()),
                (VertexKind::Action, _) => Vertex::action(path(i), keys.clone()),
                (VertexKind::Anchor, _) => Vertex::anchor(path(i)),
                (VertexKind::IndeterminateAnchor, _) => Vertex::indeterminate_anchor(path(i)),
            }
            .disrupting(disrupts.clone())
        })
        .collect();
    let edges = raw
        .1
        .iter()
        .map(|(s, t, k)| Edge::new(path(*s), path(*t), *k))
        .collect();
    let progress = raw
        .2
        .iter()
        .enumerate()
        .map(|(i, p)| (path(i), *p))
        .collect();
    (vertices, edges, progress)
}

/// Reference cycle detection: depth-first coloring over every edge kind.
fn reference_has_cycle(n: usize, edges: &[(usize, usize, EdgeKind)]) -> bool {
    fn visit(v: usize, edges: &[(usize, usize, EdgeKind)], color: &mut [u8]) -> bool {
        color[v] = 1;
        for (s, t, _) in edges {
            if *s == v {
                let c = color[*t];
                if c == 1 || (c == 0 && visit(*t, edges, color)) {
                    return true;
                }
            }
        }
        color[v] = 2;
        false
    }
    let mut color = vec![0u8; n];
    (0..n).any(|v| color[v] == 0 && visit(v, edges, &mut color))
}

/// Reference frontier: whole-graph passes to a fixed point, reading the
/// formulas of warp.md directly.
fn reference_frontier(
    graph: &Graph,
    progress: &BTreeMap<ResourceKey, Progress>,
) -> BTreeMap<ResourceKey, Resolution> {
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        Open,
        Met,
        EndedOtherwise,
        Changed,
    }
    // Each vertex's view to its dependents, and the resolution of pending ones.
    let mut view: BTreeMap<ResourceKey, S> = BTreeMap::new();
    let mut resolution: BTreeMap<ResourceKey, Resolution> = BTreeMap::new();
    for v in graph.vertices() {
        let s = match v.kind() {
            VertexKind::Anchor => S::Met,
            VertexKind::IndeterminateAnchor => S::EndedOtherwise,
            VertexKind::Action => match progress
                .get(v.resource())
                .copied()
                .unwrap_or(Progress::Pending)
            {
                Progress::Pending | Progress::Running => S::Open,
                Progress::Done(Outcome::Succeeded { changed: true }) => S::Changed,
                Progress::Done(Outcome::Succeeded { changed: false }) => S::Met,
                Progress::Done(_) => S::EndedOtherwise,
            },
        };
        view.insert(v.resource().clone(), s);
    }
    loop {
        let mut changed = false;
        for v in graph.vertices() {
            let r = v.resource();
            let pending = v.kind() == VertexKind::Action
                && matches!(
                    progress.get(r).copied().unwrap_or(Progress::Pending),
                    Progress::Pending
                );
            if !pending {
                continue;
            }
            let src = |e: &Edge| view[e.source()];
            let requires: Vec<S> = graph
                .edges_into(r)
                .filter(|e| e.kind() == EdgeKind::Requires)
                .map(src)
                .collect();
            let after: Vec<S> = graph
                .edges_into(r)
                .filter(|e| e.kind() == EdgeKind::After)
                .map(src)
                .collect();
            let group: Vec<S> = graph
                .edges_into(r)
                .filter(|e| e.kind() == EdgeKind::OnChange)
                .map(src)
                .collect();
            // warp.md: a `requires` edge or an `on_change` group whose source is
            // not met is Blocked; the group is checked before Waiting.
            let blocked =
                requires.contains(&S::EndedOtherwise) || group.contains(&S::EndedOtherwise);
            let waiting = requires
                .iter()
                .chain(&after)
                .chain(&group)
                .any(|s| *s == S::Open);
            // Reached only when every group source is met: Disabled when none
            // changed. A pending Obligation and a Variance of its own are each
            // a reason to run without a change (ADR 0009 notes), so an owed or
            // varying vertex is not Skipped.
            let own = v.is_owed() || v.varies();
            let disabled = !group.is_empty() && !group.contains(&S::Changed) && !own;
            let res = if blocked {
                Resolution::Blocked
            } else if waiting {
                Resolution::Waiting
            } else if disabled {
                Resolution::Skipped
            } else {
                Resolution::Ready
            };
            let new_view = match res {
                Resolution::Blocked => S::EndedOtherwise,
                Resolution::Skipped => S::Met,
                _ => S::Open,
            };
            if resolution.get(r) != Some(&res) {
                resolution.insert(r.clone(), res);
                changed = true;
            }
            if view[r] != new_view {
                view.insert(r.clone(), new_view);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    resolution
}

/// Law 1 (well-formedness, differential): compilation fails exactly when
/// the reference finds a cycle; on success every edge respects the order,
/// and on failure every witness is a cycle in the edge set.
#[test]
fn compilation_agrees_with_the_reference_on_cycles() {
    runner(SEEDS[0])
        .run(&raw(), |raw| {
            let (vertices, edges, _) = build(&raw);
            let cyclic = reference_has_cycle(RESOURCES, &raw.1);
            match Graph::compile(vertices, edges.clone()) {
                Ok(graph) => {
                    prop_assert!(!cyclic, "reference found a cycle the compiler missed");
                    let index: BTreeMap<&ResourceKey, usize> = graph
                        .order()
                        .iter()
                        .enumerate()
                        .map(|(i, r)| (r, i))
                        .collect();
                    for e in graph.edges() {
                        prop_assert!(index[e.source()] < index[e.target()]);
                    }
                    prop_assert_eq!(graph.order().len(), RESOURCES);
                }
                Err(CompileError::Cycle { witnesses, blocked }) => {
                    prop_assert!(cyclic, "compiler found a cycle the reference missed");
                    prop_assert!(!witnesses.is_empty());
                    let has = |s: &ResourceKey, t: &ResourceKey| {
                        edges.iter().any(|e| e.source() == s && e.target() == t)
                    };
                    for w in &witnesses {
                        for i in 0..w.len() {
                            prop_assert!(
                                has(&w[i], &w[(i + 1) % w.len()]),
                                "witness is not a cycle"
                            );
                        }
                    }
                    let cyclic_vertices: BTreeSet<&ResourceKey> =
                        witnesses.iter().flatten().collect();
                    for b in &blocked {
                        prop_assert!(!cyclic_vertices.contains(b));
                    }
                }
                Err(other) => prop_assert!(false, "unexpected error {other:?}"),
            }
            Ok(())
        })
        .unwrap();
}

/// Law 2 (N12, permutation): the compiled graph and its frontier are
/// functions of the sets, not of the order vertices or edges arrive in.
#[test]
fn compilation_and_frontier_are_invariant_under_input_permutation() {
    runner(SEEDS[1])
        .run(&raw(), |raw| {
            let (vertices, edges, progress) = build(&raw);
            let mut rv = vertices.clone();
            rv.reverse();
            let mut re = edges.clone();
            re.reverse();
            match (Graph::compile(vertices, edges), Graph::compile(rv, re)) {
                (Ok(a), Ok(b)) => {
                    prop_assert_eq!(&a, &b);
                    prop_assert_eq!(frontier(&a, &progress), frontier(&b, &progress));
                }
                (Err(a), Err(b)) => prop_assert_eq!(a, b),
                (a, b) => prop_assert!(false, "{a:?} vs {b:?}"),
            }
            Ok(())
        })
        .unwrap();
}

/// Law 3 (N4, differential): the frontier agrees with the reference on
/// every pending vertex, and a vertex resolved Ready or Skipped has every
/// `requires` and `on_change` source met: Succeeded or Skipped. A Skipped
/// vertex is met to its own dependents, so a failed or unknown trigger must
/// never produce one (ADR 0009 §1).
#[test]
fn the_frontier_agrees_with_the_reference() {
    runner(SEEDS[2])
        .run(&raw(), |raw| {
            let (vertices, edges, progress) = build(&raw);
            let Ok(graph) = Graph::compile(vertices, edges) else {
                return Ok(());
            };
            let f = frontier(&graph, &progress);
            prop_assert_eq!(f.resolutions(), &reference_frontier(&graph, &progress));
            let index: BTreeMap<&ResourceKey, usize> = graph
                .order()
                .iter()
                .enumerate()
                .map(|(i, r)| (r, i))
                .collect();
            for pair in f.ready().windows(2) {
                prop_assert!(
                    index[&pair[0]] < index[&pair[1]],
                    "ready is in topological order"
                );
            }
            let settled_well = f
                .resolutions()
                .iter()
                .filter(|(_, res)| matches!(res, Resolution::Ready | Resolution::Skipped))
                .map(|(r, _)| r);
            for r in settled_well {
                for e in graph
                    .edges_into(r)
                    .filter(|e| matches!(e.kind(), EdgeKind::Requires | EdgeKind::OnChange))
                {
                    let source = graph.vertex(e.source()).unwrap();
                    let met = match source.kind() {
                        VertexKind::Anchor => true,
                        VertexKind::IndeterminateAnchor => false,
                        VertexKind::Action => {
                            matches!(
                                progress.get(e.source()),
                                Some(Progress::Done(Outcome::Succeeded { .. }))
                            ) || f.resolution(e.source()) == Some(Resolution::Skipped)
                        }
                    };
                    prop_assert!(
                        met,
                        "{r} is {:?} with an unmet {:?} source {}",
                        f.resolution(r),
                        e.kind(),
                        e.source()
                    );
                }
            }
            Ok(())
        })
        .unwrap();
}

/// A domain for the selection law: members among four nodes, and a limit.
fn budget() -> impl Strategy<Value = (BTreeSet<Node>, usize)> {
    (nodes(), 0usize..3)
}

/// The budget predicate read from warp.md by another route: the union of
/// unavailable, reserved, and disrupted nodes, restricted to each domain.
fn fits(
    domains: &[(BTreeSet<Node>, usize)],
    unavailable: &BTreeSet<Node>,
    fresh: bool,
    reserved: &BTreeSet<Node>,
    disrupts: &BTreeSet<Node>,
) -> bool {
    if disrupts.is_empty() {
        return true;
    }
    if !fresh {
        return false;
    }
    let union: BTreeSet<&Node> = unavailable.iter().chain(reserved).chain(disrupts).collect();
    domains
        .iter()
        .all(|(members, limit)| union.iter().filter(|n| members.contains(*n)).count() <= *limit)
}

/// Law 4 (mutual exclusion, budgets, and determinism): the chosen set shares
/// no key with the held set or within itself, never exceeds capacity, keeps
/// every failure-domain budget counting what was reserved and chosen before
/// it (N9), is a prefix choice (every Ready vertex not chosen conflicts,
/// breaks a budget, or came after capacity was reached), and is the same on
/// every call.
#[test]
fn selection_is_mutually_exclusive_bounded_and_deterministic() {
    runner(SEEDS[3])
        .run(
            &(
                raw(),
                prop::collection::btree_set(0u8..3, 0..2),
                0usize..3,
                0usize..5,
                (
                    prop::collection::vec(budget(), 0..3),
                    nodes(),
                    prop::bool::weighted(0.8),
                    nodes(),
                ),
            ),
            |(raw, held, reserved, capacity, (domains, unavailable, fresh, reserved_nodes))| {
                let (vertices, edges, progress) = build(&raw);
                let Ok(graph) = Graph::compile(vertices, edges) else {
                    return Ok(());
                };
                let held: BTreeSet<ConflictKey> = held.into_iter().map(key).collect();
                let budgets = Budgets::new(
                    domains
                        .iter()
                        .map(|(m, k)| Budget::new(m.clone(), *k))
                        .collect(),
                    unavailable.clone(),
                    fresh,
                );
                let before = Reserved {
                    keys: held.clone(),
                    nodes: reserved_nodes.clone(),
                    count: reserved,
                };
                let ready = frontier(&graph, &progress);
                let s = select(&graph, &ready, &before, capacity, &budgets);
                prop_assert_eq!(&s, &select(&graph, &ready, &before, capacity, &budgets));
                prop_assert!(reserved + s.chosen().len() <= capacity.max(reserved));
                let mut seen = held.clone();
                for c in s.chosen() {
                    let keys = graph.vertex(c).unwrap().keys();
                    prop_assert!(
                        keys.is_disjoint(&seen),
                        "conflicting Actions selected together"
                    );
                    seen.extend(keys.iter().cloned());
                }
                prop_assert_eq!(&seen, s.held());
                let mut running = held.clone();
                let mut disrupted = reserved_nodes.clone();
                let mut count = reserved;
                for r in ready.ready() {
                    let v = graph.vertex(r).unwrap();
                    let chosen = s.chosen().contains(r);
                    if count >= capacity {
                        prop_assert!(!chosen, "chosen past capacity");
                        continue;
                    }
                    let free = v.keys().is_disjoint(&running);
                    let within = fits(&domains, &unavailable, fresh, &disrupted, v.disrupts());
                    if free && within {
                        prop_assert!(chosen, "{r} fits and was not chosen");
                        running.extend(v.keys().iter().cloned());
                        disrupted.extend(v.disrupts().iter().cloned());
                        count += 1;
                    } else {
                        prop_assert!(!chosen, "{r} conflicts or breaks a budget and was chosen");
                    }
                }
                prop_assert_eq!(&disrupted, s.nodes());
                Ok(())
            },
        )
        .unwrap();
}
