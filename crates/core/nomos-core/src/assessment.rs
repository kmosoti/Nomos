//! The assessment algebra: a Condition and its Observations produce exactly
//! one Assessment (spec §3, §8; ADR 0005).
//!
//! The rule that governs everything here: **unknown evidence does not imply
//! noncompliance** (N13). A failed collection, a resource nobody observed,
//! and two collectors that contradict each other are Indeterminate, with the
//! reason. Only sufficient evidence produces Satisfied or a Variance, and
//! [`assess_evidence`], the function that judges evidence, cannot return
//! Indeterminate at all: it has no reason to give.
//!
//! Sufficient means ([reconciliation.md](../../../../docs/formal/reconciliation.md)):
//! at least one Observation of the resource was collected, and every
//! collected Observation agrees. A collector that failed contributes no
//! evidence and does not contradict one that succeeded; whether its failure
//! should still count against sufficiency is a policy the transition kernel
//! decides, not this function. Freshness is likewise a policy applied to the
//! collection window before assessment, never inside it: a matching
//! Observation is Satisfied whatever its age.
//!
//! A [`Report`] keeps every Assessment. An Indeterminate Assessment of one
//! resource never erases the Variance of another, and the Plan takes its
//! input from [`Report::variances`] alone.

use alloc::vec::Vec;

use crate::condition::{Condition, Content, FileCondition};
use crate::observation::{Collection, CollectionFailure, FileEvidence, Observation};
use crate::resource::Digest;

/// A known mismatch between a Condition and sufficient evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Variance {
    /// The Condition requires a file and none exists.
    Missing,
    /// The Condition requires no file and one exists.
    Unexpected,
    /// The file exists with the wrong content.
    ContentDiffers {
        /// The digest the Condition requires.
        expected: Digest,
        /// The digest observed.
        observed: Digest,
    },
}

/// Why evidence was insufficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// No Observation of the resource was supplied.
    NoObservation,
    /// Every Observation of the resource failed. When they failed for
    /// different reasons this is the smallest under [`CollectionFailure`]'s
    /// ordering, so that the Assessment does not depend on the order the
    /// Observations arrived in.
    CollectionFailed(CollectionFailure),
    /// Collected Observations of the resource disagree.
    Conflicting,
}

/// The interpretation of a Condition against evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Assessment {
    /// Sufficient evidence, and the Condition holds.
    Satisfied,
    /// Sufficient evidence, and the Condition does not hold.
    Variance(Variance),
    /// Insufficient or failed evidence. Never a mismatch.
    Indeterminate(Reason),
}

impl Assessment {
    /// Whether this is a Variance.
    pub fn is_variance(&self) -> bool {
        matches!(self, Assessment::Variance(_))
    }

    /// Whether this is Indeterminate.
    pub fn is_indeterminate(&self) -> bool {
        matches!(self, Assessment::Indeterminate(_))
    }
}

/// Judges collected evidence against a requirement. Evidence is sufficient
/// by construction here, so the result is Satisfied or a Variance, never
/// Indeterminate.
pub fn assess_evidence(requirement: &FileCondition, evidence: &FileEvidence) -> Assessment {
    match (requirement, evidence) {
        (FileCondition::Absent, FileEvidence::Absent) => Assessment::Satisfied,
        (FileCondition::Absent, FileEvidence::Present { .. }) => {
            Assessment::Variance(Variance::Unexpected)
        }
        (FileCondition::Present { .. }, FileEvidence::Absent) => {
            Assessment::Variance(Variance::Missing)
        }
        (
            FileCondition::Present {
                content: Content::Any,
            },
            FileEvidence::Present { .. },
        ) => Assessment::Satisfied,
        (
            FileCondition::Present {
                content: Content::Exactly(expected),
            },
            FileEvidence::Present { digest, .. },
        ) => {
            if digest == expected {
                Assessment::Satisfied
            } else {
                Assessment::Variance(Variance::ContentDiffers {
                    expected: *expected,
                    observed: *digest,
                })
            }
        }
    }
}

/// Judges one collection outcome: a failure is Indeterminate with its
/// reason, and evidence goes to [`assess_evidence`].
pub fn assess_collection(requirement: &FileCondition, collection: &Collection) -> Assessment {
    match collection {
        Collection::Collected(evidence) => assess_evidence(requirement, evidence),
        Collection::Failed(failure) => {
            Assessment::Indeterminate(Reason::CollectionFailed(*failure))
        }
    }
}

/// Assesses `condition` against every Observation of its resource among
/// `observations`. Observations of other resources are ignored.
pub fn assess(condition: &Condition, observations: &[Observation]) -> Assessment {
    let relevant = observations.iter().filter(|o| o.path() == condition.path());
    let mut evidence: Option<FileEvidence> = None;
    let mut least_failure: Option<CollectionFailure> = None;
    let mut seen = false;
    for observation in relevant {
        seen = true;
        match observation.collection() {
            Collection::Collected(this) => match evidence {
                None => evidence = Some(*this),
                Some(that) if that == *this => {}
                Some(_) => return Assessment::Indeterminate(Reason::Conflicting),
            },
            Collection::Failed(failure) => {
                least_failure = Some(least_failure.map_or(*failure, |f| f.min(*failure)));
            }
        }
    }
    match (seen, evidence, least_failure) {
        (false, _, _) => Assessment::Indeterminate(Reason::NoObservation),
        (true, Some(evidence), _) => assess_evidence(condition.requirement(), &evidence),
        (true, None, Some(failure)) => Assessment::Indeterminate(Reason::CollectionFailed(failure)),
        // Seen implies evidence or a failure; the arm is unreachable by construction.
        (true, None, None) => Assessment::Indeterminate(Reason::NoObservation),
    }
}

/// Every Assessment of a Canon's Conditions, kept per Condition, in a
/// deterministic order (by resource, then requirement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    entries: Vec<(Condition, Assessment)>,
}

impl Report {
    /// Assesses each Condition against the Observations.
    pub fn assess(conditions: &[Condition], observations: &[Observation]) -> Self {
        let mut entries: Vec<(Condition, Assessment)> = conditions
            .iter()
            .map(|c| (c.clone(), assess(c, observations)))
            .collect();
        entries.sort();
        Report { entries }
    }

    /// Every Condition with its Assessment.
    pub fn entries(&self) -> &[(Condition, Assessment)] {
        &self.entries
    }

    /// The Plan's input: every Variance, and nothing else (N13).
    pub fn variances(&self) -> impl Iterator<Item = (&Condition, &Variance)> {
        self.entries.iter().filter_map(|(c, a)| match a {
            Assessment::Variance(v) => Some((c, v)),
            _ => None,
        })
    }

    /// Every Indeterminate Assessment with its reason, for the report to
    /// the operator. None of these reaches the Plan.
    pub fn indeterminate(&self) -> impl Iterator<Item = (&Condition, &Reason)> {
        self.entries.iter().filter_map(|(c, a)| match a {
            Assessment::Indeterminate(r) => Some((c, r)),
            _ => None,
        })
    }

    /// Whether every Assessment is Satisfied. Convergence needs more than
    /// this (discharged Obligations, Settled effects); this is the
    /// Assessment half.
    pub fn all_satisfied(&self) -> bool {
        self.entries
            .iter()
            .all(|(_, a)| matches!(a, Assessment::Satisfied))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{Assessment, Reason, Report, Variance, assess, assess_collection, assess_evidence};
    use crate::condition::{Condition, Content, FileCondition};
    use crate::observation::{
        Collection, CollectionFailure, CollectorId, FileEvidence, Instant, Observation, Provenance,
        Window,
    };
    use crate::resource::{Digest, ResourcePath};

    fn path(text: &str) -> ResourcePath {
        ResourcePath::new(text).unwrap()
    }

    fn digest(byte: u8) -> Digest {
        Digest::from_bytes([byte; 32])
    }

    fn provenance(end: u64) -> Provenance {
        Provenance::new(
            CollectorId::new("mock").unwrap(),
            Window::new(Instant(0), Instant(end)).unwrap(),
        )
    }

    fn observed(text: &str, collection: Collection) -> Observation {
        Observation::file(path(text), collection, provenance(10))
    }

    const FAILURES: [CollectionFailure; 4] = [
        CollectionFailure::PermissionDenied,
        CollectionFailure::TimedOut,
        CollectionFailure::Unsupported,
        CollectionFailure::Io,
    ];

    fn requirements() -> [FileCondition; 3] {
        [
            FileCondition::Absent,
            FileCondition::Present {
                content: Content::Any,
            },
            FileCondition::Present {
                content: Content::Exactly(digest(1)),
            },
        ]
    }

    fn evidences() -> [FileEvidence; 3] {
        [
            FileEvidence::Absent,
            FileEvidence::Present {
                digest: digest(1),
                size: 3,
            },
            FileEvidence::Present {
                digest: digest(2),
                size: 3,
            },
        ]
    }

    /// The exhaustive pair table: every requirement against every evidence.
    #[test]
    fn the_evidence_truth_table_is_exhaustive_and_never_indeterminate() {
        let [absent, any, exactly] = requirements();
        let [none, one, two] = evidences();
        let table = [
            (absent, none, Assessment::Satisfied),
            (absent, one, Assessment::Variance(Variance::Unexpected)),
            (absent, two, Assessment::Variance(Variance::Unexpected)),
            (any, none, Assessment::Variance(Variance::Missing)),
            (any, one, Assessment::Satisfied),
            (any, two, Assessment::Satisfied),
            (exactly, none, Assessment::Variance(Variance::Missing)),
            (exactly, one, Assessment::Satisfied),
            (
                exactly,
                two,
                Assessment::Variance(Variance::ContentDiffers {
                    expected: digest(1),
                    observed: digest(2),
                }),
            ),
        ];
        assert_eq!(table.len(), requirements().len() * evidences().len());
        for (requirement, evidence, expected) in table {
            let actual = assess_evidence(&requirement, &evidence);
            assert_eq!(actual, expected, "{requirement:?} vs {evidence:?}");
            assert!(!actual.is_indeterminate());
        }
    }

    /// Every failure, against every requirement, is Indeterminate with that failure.
    #[test]
    fn a_failed_observation_is_indeterminate() {
        for requirement in requirements() {
            for failure in FAILURES {
                let condition = Condition::file(path("/etc/hosts"), requirement);
                let observation = observed("/etc/hosts", Collection::Failed(failure));
                let expected = Assessment::Indeterminate(Reason::CollectionFailed(failure));
                assert_eq!(
                    assess_collection(&requirement, &Collection::Failed(failure)),
                    expected
                );
                assert_eq!(assess(&condition, &[observation]), expected);
            }
        }
    }

    #[test]
    fn a_denied_read_is_neither_satisfied_nor_variance() {
        let condition = Condition::file(path("/etc/shadow"), FileCondition::Absent);
        let a = assess(
            &condition,
            &[observed(
                "/etc/shadow",
                Collection::Failed(CollectionFailure::PermissionDenied),
            )],
        );
        assert!(a.is_indeterminate());
        assert!(!a.is_variance());
        assert_ne!(a, Assessment::Satisfied);
    }

    #[test]
    fn a_resource_nobody_observed_is_indeterminate() {
        let condition = Condition::file(path("/etc/hosts"), FileCondition::Absent);
        assert_eq!(
            assess(&condition, &[]),
            Assessment::Indeterminate(Reason::NoObservation)
        );
        let other = observed("/etc/motd", Collection::Collected(FileEvidence::Absent));
        assert_eq!(
            assess(&condition, &[other]),
            Assessment::Indeterminate(Reason::NoObservation),
            "an Observation of another resource is not evidence"
        );
    }

    #[test]
    fn contradicting_observations_are_indeterminate() {
        let condition = Condition::file(
            path("/etc/hosts"),
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let [_, one, two] = evidences();
        let a = assess(
            &condition,
            &[
                observed("/etc/hosts", Collection::Collected(one)),
                observed("/etc/hosts", Collection::Collected(two)),
            ],
        );
        assert_eq!(a, Assessment::Indeterminate(Reason::Conflicting));
    }

    #[test]
    fn agreeing_observations_are_one_piece_of_evidence() {
        let condition = Condition::file(path("/etc/hosts"), FileCondition::Absent);
        let a = assess(
            &condition,
            &[
                observed("/etc/hosts", Collection::Collected(FileEvidence::Absent)),
                observed("/etc/hosts", Collection::Collected(FileEvidence::Absent)),
            ],
        );
        assert_eq!(a, Assessment::Satisfied);
    }

    #[test]
    fn a_failure_beside_evidence_does_not_contradict_it() {
        let condition = Condition::file(path("/etc/hosts"), FileCondition::Absent);
        let a = assess(
            &condition,
            &[
                observed(
                    "/etc/hosts",
                    Collection::Failed(CollectionFailure::TimedOut),
                ),
                observed("/etc/hosts", Collection::Collected(FileEvidence::Absent)),
            ],
        );
        assert_eq!(a, Assessment::Satisfied);
    }

    /// Freshness is a policy on the window, applied before assessment; the
    /// algebra itself does not read the clock.
    #[test]
    fn a_matching_observation_is_satisfied_whatever_its_age() {
        let condition = Condition::file(path("/etc/hosts"), FileCondition::Absent);
        for end in [0, 1, u64::MAX] {
            let observation = Observation::file(
                path("/etc/hosts"),
                Collection::Collected(FileEvidence::Absent),
                Provenance::new(
                    CollectorId::new("mock").unwrap(),
                    Window::new(Instant(0), Instant(end)).unwrap(),
                ),
            );
            assert_eq!(
                assess(&condition, &[observation]),
                Assessment::Satisfied,
                "window ending at {end}"
            );
        }
    }

    /// Counterexample `partial-assessment`: a Variance beside an unrelated
    /// Indeterminate is still reported.
    #[test]
    fn an_indeterminate_assessment_does_not_hide_a_variance() {
        let conditions = vec![
            Condition::file(path("/etc/hosts"), FileCondition::Absent),
            Condition::file(path("/etc/shadow"), FileCondition::Absent),
        ];
        let observations = vec![
            observed(
                "/etc/hosts",
                Collection::Collected(FileEvidence::Present {
                    digest: digest(1),
                    size: 1,
                }),
            ),
            observed(
                "/etc/shadow",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
        ];
        let report = Report::assess(&conditions, &observations);
        assert_eq!(report.entries().len(), 2);
        let variances: Vec<_> = report.variances().collect();
        assert_eq!(variances.len(), 1);
        assert_eq!(variances[0].0.path().as_str(), "/etc/hosts");
        assert_eq!(*variances[0].1, Variance::Unexpected);
        let indeterminate: Vec<_> = report.indeterminate().collect();
        assert_eq!(indeterminate.len(), 1);
        assert_eq!(indeterminate[0].0.path().as_str(), "/etc/shadow");
        assert!(!report.all_satisfied());
    }

    /// N13: the Plan's input is every Variance and nothing else.
    #[test]
    fn indeterminate_assessments_do_not_reach_the_plan() {
        let conditions = vec![
            Condition::file(path("/a"), FileCondition::Absent),
            Condition::file(path("/b"), FileCondition::Absent),
            Condition::file(path("/c"), FileCondition::Absent),
        ];
        let observations = vec![
            observed("/a", Collection::Failed(CollectionFailure::Io)),
            observed(
                "/b",
                Collection::Collected(FileEvidence::Present {
                    digest: digest(9),
                    size: 0,
                }),
            ),
        ];
        let report = Report::assess(&conditions, &observations);
        let plan_input: Vec<&str> = report.variances().map(|(c, _)| c.path().as_str()).collect();
        assert_eq!(plan_input, ["/b"]);
        let reasons: Vec<(&str, Reason)> = report
            .indeterminate()
            .map(|(c, r)| (c.path().as_str(), *r))
            .collect();
        assert_eq!(
            reasons,
            [
                ("/a", Reason::CollectionFailed(CollectionFailure::Io)),
                ("/c", Reason::NoObservation),
            ]
        );
    }

    /// Counterexample fixture.
    /// found_by: property test `laws::a_report_is_invariant_under_input_permutation`
    /// seed: `SEEDS[3]` of `tests/laws.rs` (`4e313304` then zeros), proptest 1.11, ChaCha
    /// input: the two Observations below, in either order
    /// property: N12 (the Report is a function of the sets, not their order)
    /// fix: `assess` reports the least failure instead of the first one seen
    #[test]
    fn two_failures_give_the_same_reason_in_either_order() {
        let condition = Condition::file(path("/r/0"), FileCondition::Absent);
        let denied = Observation::file(
            path("/r/0"),
            Collection::Failed(CollectionFailure::PermissionDenied),
            provenance(0),
        );
        let timed_out = Observation::file(
            path("/r/0"),
            Collection::Failed(CollectionFailure::TimedOut),
            provenance(0),
        );
        let expected = Assessment::Indeterminate(Reason::CollectionFailed(
            CollectionFailure::PermissionDenied,
        ));
        assert_eq!(
            assess(&condition, &[denied.clone(), timed_out.clone()]),
            expected
        );
        assert_eq!(assess(&condition, &[timed_out, denied]), expected);
    }

    #[test]
    fn a_report_is_ordered_by_resource_whatever_the_input_order() {
        let a = Condition::file(path("/a"), FileCondition::Absent);
        let b = Condition::file(path("/b"), FileCondition::Absent);
        let observations = vec![
            observed("/b", Collection::Collected(FileEvidence::Absent)),
            observed("/a", Collection::Collected(FileEvidence::Absent)),
        ];
        let forward = Report::assess(&[a.clone(), b.clone()], &observations);
        let backward = Report::assess(&[b, a], &observations);
        assert_eq!(forward, backward);
        assert!(forward.all_satisfied());
    }
}
