//! Algebraic laws of the assessment algebra, checked on generated inputs.
//!
//! The runner is seeded from a fixed seed bank, so every run explores the
//! same inputs and a failure reproduces from the seed alone. A new law gets
//! a new seed, recorded here; a changed generator changes what a seed
//! produces, which is why a found counterexample is promoted to a concrete
//! fixture rather than kept as a seed (verification strategy, Counterexamples
//! Become Fixtures). The laws are stated from spec §3 and N13, not from the
//! implementation: their oracle is the invariant, so they count as evidence
//! for it (ADR 0015 §5).

use std::collections::BTreeSet;

use nomos_core::assessment::{Assessment, Reason, Report, assess, assess_file};
use nomos_core::condition::{Condition, Content, FileCondition};
use nomos_core::observation::{
    Account, Collection, CollectionFailure, CollectorId, FileEvidence, Instant, Observation,
    ObservedMetadata, Provenance, Window,
};
use nomos_core::resource::{Digest, Mode, ResourcePath};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

/// The seed bank. One 32-byte seed per law; the bytes are arbitrary and
/// fixed forever.
const SEEDS: [[u8; 32]; 5] = [
    [
        0x4e, 0x31, 0x33, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0,
    ],
    [
        0x4e, 0x31, 0x33, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0,
    ],
    [
        0x4e, 0x31, 0x33, 0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0,
    ],
    [
        0x4e, 0x31, 0x33, 0x04, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0,
    ],
    [
        0x4e, 0x31, 0x33, 0x05, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0,
    ],
];

const CASES: u32 = 512;

fn runner(seed: [u8; 32]) -> TestRunner {
    let config = Config {
        cases: CASES,
        failure_persistence: None,
        ..Config::default()
    };
    TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &seed))
}

fn digest() -> impl Strategy<Value = Digest> {
    // Two digests are enough to make matches and mismatches both likely.
    prop_oneof![
        Just(Digest::from_bytes([1; 32])),
        Just(Digest::from_bytes([2; 32]))
    ]
}

fn requirement() -> impl Strategy<Value = FileCondition> {
    prop_oneof![
        Just(FileCondition::Absent),
        Just(FileCondition::present(Content::Any)),
        digest().prop_map(|d| FileCondition::present(Content::Exactly(d))),
    ]
}

/// The laws are about content; the metadata is fixed, and the requirement
/// states none, so it is never a Variance here.
fn metadata() -> ObservedMetadata {
    ObservedMetadata {
        owner: Account::Id(0),
        group: Account::Id(0),
        mode: Mode::new(0o644).unwrap(),
    }
}

fn evidence() -> impl Strategy<Value = FileEvidence> {
    prop_oneof![
        Just(FileEvidence::Absent),
        (digest(), 0u64..4).prop_map(|(digest, size)| FileEvidence::Present {
            digest,
            size,
            metadata: metadata(),
        }),
    ]
}

fn failure() -> impl Strategy<Value = CollectionFailure> {
    prop_oneof![
        Just(CollectionFailure::PermissionDenied),
        Just(CollectionFailure::TimedOut),
        Just(CollectionFailure::Unsupported),
        Just(CollectionFailure::Io),
        Just(CollectionFailure::Unavailable),
    ]
}

fn collection() -> impl Strategy<Value = Collection<FileEvidence>> {
    prop_oneof![
        evidence().prop_map(Collection::Collected),
        failure().prop_map(Collection::Failed),
    ]
}

/// A small universe of resources, so that several Conditions and
/// Observations land on the same one.
fn path() -> impl Strategy<Value = ResourcePath> {
    (0u8..4).prop_map(|n| ResourcePath::new(&format!("/r/{n}")).unwrap())
}

fn provenance() -> impl Strategy<Value = Provenance> {
    (0u64..100, 0u64..100).prop_map(|(a, b)| {
        Provenance::new(
            CollectorId::new("gen").unwrap(),
            Window::new(Instant(a.min(b)), Instant(a.max(b))).unwrap(),
        )
    })
}

fn observation() -> impl Strategy<Value = Observation> {
    (path(), collection(), provenance()).prop_map(|(p, c, v)| Observation::file(p, c, v))
}

fn condition() -> impl Strategy<Value = Condition> {
    (path(), requirement()).prop_map(|(p, r)| Condition::file(p, r))
}

/// The reference reading of spec §3, independent of the implementation.
fn holds(requirement: &FileCondition, evidence: &FileEvidence) -> bool {
    match requirement {
        FileCondition::Absent => matches!(evidence, FileEvidence::Absent),
        FileCondition::Present {
            content: Content::Any,
            ..
        } => matches!(evidence, FileEvidence::Present { .. }),
        FileCondition::Present {
            content: Content::Exactly(expected),
            ..
        } => matches!(evidence, FileEvidence::Present { digest, .. } if digest == expected),
    }
}

/// Law 1 (soundness, reconciliation.md): on collected evidence, Satisfied
/// exactly when the evidence satisfies the requirement, never Indeterminate.
#[test]
fn evidence_assessment_is_sound_on_generated_inputs() {
    runner(SEEDS[0])
        .run(&(requirement(), evidence()), |(r, e)| {
            let a = assess_file(&r, &e);
            prop_assert_eq!(matches!(a, Assessment::Satisfied), holds(&r, &e));
            prop_assert!(!a.is_indeterminate());
            Ok(())
        })
        .unwrap();
}

/// Law 2 (N13): a Condition whose every Observation failed is Indeterminate
/// with the least failure, for every requirement and every failure sequence.
#[test]
fn only_failures_is_indeterminate_with_the_least_failure() {
    runner(SEEDS[1])
        .run(
            &(
                requirement(),
                prop::collection::vec((failure(), provenance()), 1..4),
            ),
            |(r, failures)| {
                let path = ResourcePath::new("/r/0").unwrap();
                let condition = Condition::file(path.clone(), r);
                let observations: Vec<Observation> = failures
                    .iter()
                    .map(|(f, v)| {
                        Observation::file(path.clone(), Collection::Failed(*f), v.clone())
                    })
                    .collect();
                let least = failures.iter().map(|(f, _)| *f).min().unwrap();
                let a = assess(&condition, &observations);
                prop_assert_eq!(
                    a,
                    Assessment::Indeterminate(Reason::CollectionFailed(least))
                );
                Ok(())
            },
        )
        .unwrap();
}

/// Law 3 (N13, monotonicity): the Plan's input never contains an
/// Indeterminate Assessment, and adding a Condition whose evidence is
/// insufficient adds nothing to it.
#[test]
fn indeterminate_assessments_never_reach_the_plan_input() {
    runner(SEEDS[2])
        .run(
            &(
                prop::collection::vec(condition(), 0..6),
                prop::collection::vec(observation(), 0..8),
                requirement(),
            ),
            |(conditions, observations, extra)| {
                let report = Report::assess(&conditions, &observations);
                for (condition, _) in report.variances() {
                    let a = assess(condition, &observations);
                    prop_assert!(a.is_variance());
                }
                let plan_input: Vec<_> = report.variances().map(|(c, v)| (c.clone(), *v)).collect();
                // A Condition on a resource nobody observed is Indeterminate.
                let unobserved = Condition::file(ResourcePath::new("/nowhere").unwrap(), extra);
                let mut more = conditions.clone();
                more.push(unobserved.clone());
                let wider = Report::assess(&more, &observations);
                prop_assert_eq!(
                    assess(&unobserved, &observations),
                    Assessment::Indeterminate(Reason::NoObservation)
                );
                let wider_input: Vec<_> = wider.variances().map(|(c, v)| (c.clone(), *v)).collect();
                prop_assert_eq!(wider_input, plan_input);
                Ok(())
            },
        )
        .unwrap();
}

/// Law 4 (N12, permutation): a Report is a function of the sets, not of the
/// order Conditions or Observations arrive in.
#[test]
fn a_report_is_invariant_under_input_permutation() {
    runner(SEEDS[3])
        .run(
            &(
                prop::collection::vec(condition(), 0..6),
                prop::collection::vec(observation(), 0..8),
            ),
            |(conditions, observations)| {
                let reference = Report::assess(&conditions, &observations);
                let mut rc = conditions.clone();
                rc.reverse();
                let mut ro = observations.clone();
                ro.reverse();
                prop_assert_eq!(Report::assess(&rc, &observations), reference.clone());
                prop_assert_eq!(Report::assess(&conditions, &ro), reference.clone());
                prop_assert_eq!(Report::assess(&rc, &ro), reference);
                Ok(())
            },
        )
        .unwrap();
}

/// Law 5 (sufficiency): with at least one collected Observation, the result
/// is Conflicting exactly when the collected evidence disagrees, and
/// otherwise the judgment of that one piece of evidence.
#[test]
fn collected_evidence_is_judged_when_it_agrees_and_indeterminate_when_it_does_not() {
    runner(SEEDS[4])
        .run(
            &(
                requirement(),
                prop::collection::vec((collection(), provenance()), 1..5),
            ),
            |(r, collections)| {
                let path = ResourcePath::new("/r/1").unwrap();
                let condition = Condition::file(path.clone(), r.clone());
                let observations: Vec<Observation> = collections
                    .iter()
                    .map(|(c, v)| Observation::file(path.clone(), c.clone(), v.clone()))
                    .collect();
                let evidences: BTreeSet<FileEvidence> = collections
                    .iter()
                    .filter_map(|(c, _)| match c {
                        Collection::Collected(e) => Some(e.clone()),
                        Collection::Failed(_) => None,
                    })
                    .collect();
                let a = assess(&condition, &observations);
                match evidences.len() {
                    0 => prop_assert!(a.is_indeterminate()),
                    1 => {
                        let e = evidences.iter().next().unwrap();
                        prop_assert_eq!(a, assess_file(&r, e));
                    }
                    _ => prop_assert_eq!(a, Assessment::Indeterminate(Reason::Conflicting)),
                }
                Ok(())
            },
        )
        .unwrap();
}
