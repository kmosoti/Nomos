//! Bounded-verifier harnesses, compiled only under `cargo kani`.
//!
//! Each harness states a property of a pure resolution predicate over every
//! value of its argument types, with cover checks so that a vacuous harness
//! is reported. The graph walk in `frontier` is not harnessed; its
//! agreement with the predicates is the differential law in `tests/laws.rs`.

use crate::frontier::{
    AfterState, EdgeState, GroupState, Outcome, Resolution, SourceState, resolve_after,
    resolve_group, resolve_requires, resolve_vertex,
};

/// A `requires` edge is Satisfied exactly when the source succeeded or was
/// skipped, Waiting exactly when the source is open, and Blocked otherwise;
/// an `after` edge never blocks and is Satisfied exactly when the source is
/// terminal (N4, ADR 0009 §2).
#[kani::proof]
fn edge_resolution_matches_the_table() {
    let source: SourceState = kani::any();
    let requires = resolve_requires(source);
    let after = resolve_after(source);
    let met = matches!(
        source,
        SourceState::Done(Outcome::Succeeded { .. }) | SourceState::Skipped
    );
    assert_eq!(requires == EdgeState::Satisfied, met);
    assert_eq!(requires == EdgeState::Waiting, source == SourceState::Open);
    assert_eq!(after == AfterState::Satisfied, source != SourceState::Open);
    kani::cover!(requires == EdgeState::Blocked);
    kani::cover!(requires == EdgeState::Satisfied);
    kani::cover!(after == AfterState::Waiting);
}

/// A group of up to three sources is Blocked if any source is terminal and
/// not met, else Waiting if any is open, else Activated if any succeeded with
/// a change, else Disabled. So it is never Activated by a source that did not
/// change (counterexample `activation-missing`), and never Activated or
/// Disabled beside a source that failed or whose outcome is unknown.
#[kani::proof]
fn group_resolution_matches_the_table() {
    let sources: [SourceState; 3] = kani::any();
    let group = resolve_group(&sources);
    let met = |s: &SourceState| {
        matches!(
            s,
            SourceState::Done(Outcome::Succeeded { .. }) | SourceState::Skipped
        )
    };
    let any_unmet = sources.iter().any(|s| *s != SourceState::Open && !met(s));
    let any_open = sources.iter().any(|s| *s == SourceState::Open);
    let any_changed = sources
        .iter()
        .any(|s| *s == SourceState::Done(Outcome::Succeeded { changed: true }));
    assert_ne!(group, GroupState::Empty);
    assert_eq!(group == GroupState::Blocked, any_unmet);
    assert_eq!(group == GroupState::Waiting, !any_unmet && any_open);
    assert_eq!(
        group == GroupState::Activated,
        !any_unmet && !any_open && any_changed
    );
    assert_eq!(
        group == GroupState::Disabled,
        !any_unmet && !any_open && !any_changed
    );
    kani::cover!(group == GroupState::Activated);
    kani::cover!(group == GroupState::Disabled);
    kani::cover!(group == GroupState::Blocked && any_changed);
    kani::cover!(group == GroupState::Blocked && any_open);
}

/// A vertex is Ready only when every `requires` edge is Satisfied (N4),
/// every `after` edge is Satisfied, and its group is Empty or Activated; it
/// is Skipped only when every edge is Satisfied and its group is Disabled;
/// and it is Blocked exactly when a `requires` edge or its group is.
#[kani::proof]
fn vertex_resolution_is_ready_only_when_startable_and_activated() {
    let requires: [EdgeState; 2] = kani::any();
    let after: [AfterState; 2] = kani::any();
    let group: GroupState = kani::any();
    let resolution = resolve_vertex(&requires, &after, group);
    let any_blocked = requires.contains(&EdgeState::Blocked) || group == GroupState::Blocked;
    let all_met = requires.iter().all(|e| *e == EdgeState::Satisfied)
        && after.iter().all(|e| *e == AfterState::Satisfied);
    assert_eq!(resolution == Resolution::Blocked, any_blocked);
    if resolution == Resolution::Ready {
        assert!(all_met);
        assert!(matches!(group, GroupState::Empty | GroupState::Activated));
    }
    if resolution == Resolution::Skipped {
        assert!(all_met && group == GroupState::Disabled);
    }
    kani::cover!(resolution == Resolution::Ready);
    kani::cover!(resolution == Resolution::Skipped);
    kani::cover!(resolution == Resolution::Waiting);
}
