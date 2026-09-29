/// Judges collected evidence against a requirement. Evidence is sufficient
/// by construction here, so the result is Satisfied or a Variance, never
/// Indeterminate.
pub fn assess_evidence(requirement: &FileCondition, evidence: &FileEvidence) -> Assessment {
    match (*requirement, *evidence) {
        (FileCondition::Absent, FileEvidence::Absent) => Assessment::Satisfied,
        (FileCondition::Absent, FileEvidence::Present { .. }) => {
            Assessment::Variance(Variance::Unexpected)
        }
        (FileCondition::Present { .. }, FileEvidence::Absent) => {
            Assessment::Variance(Variance::Missing)
        }
        (FileCondition::Present { content }, FileEvidence::Present { digest, .. }) => {
            assess_content(content, digest)
        }
    }
}

/// Judges the digest of a present file against a content requirement.
fn assess_content(content: Content, observed: Digest) -> Assessment {
    match content {
        Content::Any => Assessment::Satisfied,
        Content::Exactly(expected) if expected == observed => Assessment::Satisfied,
        Content::Exactly(expected) => {
            Assessment::Variance(Variance::ContentDiffers { expected, observed })
        }
    }
}

/// Judges one collection outcome: a failure is Indeterminate with its
/// reason, and evidence goes to [`assess_evidence`].
pub fn assess_collection(requirement: &FileCondition, collection: &Collection) -> Assessment {
    match collection {
        Collection::Failed(failure) => {
            Assessment::Indeterminate(Reason::CollectionFailed(*failure))
        }
        Collection::Collected(evidence) => assess_evidence(requirement, evidence),
    }
}

/// Assesses `condition` against every Observation of its resource among
/// `observations`. Observations of other resources are ignored.
pub fn assess(condition: &Condition, observations: &[Observation]) -> Assessment {
    let relevant: Vec<&Collection> = observations
        .iter()
        .filter(|observation| observation.path() == condition.path())
        .map(Observation::collection)
        .collect();

    // The single piece of evidence every collected Observation agrees on,
    // and the smallest failure among the failed ones. Both are independent
    // of the order the Observations arrived in.
    let mut agreed: Option<FileEvidence> = None;
    let mut smallest_failure: Option<CollectionFailure> = None;
    for collection in relevant {
        match *collection {
            Collection::Collected(evidence) => match agreed {
                None => agreed = Some(evidence),
                Some(seen) if seen == evidence => {}
                Some(_) => return Assessment::Indeterminate(Reason::Conflicting),
            },
            Collection::Failed(failure) => {
                smallest_failure = Some(match smallest_failure {
                    Some(previous) if previous <= failure => previous,
                    _ => failure,
                });
            }
        }
    }

    match (agreed, smallest_failure) {
        (Some(evidence), _) => assess_evidence(condition.requirement(), &evidence),
        (None, Some(failure)) => Assessment::Indeterminate(Reason::CollectionFailed(failure)),
        (None, None) => Assessment::Indeterminate(Reason::NoObservation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use alloc::vec;

    use crate::observation::{CollectorId, Instant, Provenance, Window};
    use crate::resource::ResourcePath;

    const FAILURES: [CollectionFailure; 4] = [
        CollectionFailure::PermissionDenied,
        CollectionFailure::TimedOut,
        CollectionFailure::Unsupported,
        CollectionFailure::Io,
    ];

    fn digest(byte: u8) -> Digest {
        Digest::from_bytes([byte; 32])
    }

    fn path(text: &str) -> ResourcePath {
        ResourcePath::new(text).unwrap()
    }

    fn provenance(collector: &str, at: u64) -> Provenance {
        Provenance::new(
            CollectorId::new(collector).unwrap(),
            Window::new(Instant(at), Instant(at + 1)).unwrap(),
        )
    }

    fn observe(at: &str, collection: Collection) -> Observation {
        Observation::file(path(at), collection, provenance("test", 0))
    }

    fn present(byte: u8) -> FileEvidence {
        FileEvidence::Present {
            digest: digest(byte),
            size: 10,
        }
    }

    fn requirements() -> Vec<FileCondition> {
        vec![
            FileCondition::Absent,
            FileCondition::Present {
                content: Content::Any,
            },
            FileCondition::Present {
                content: Content::Exactly(digest(1)),
            },
            FileCondition::Present {
                content: Content::Exactly(digest(2)),
            },
        ]
    }

    fn evidences() -> Vec<FileEvidence> {
        vec![FileEvidence::Absent, present(1), present(2)]
    }

    fn collections() -> Vec<Collection> {
        let mut all: Vec<Collection> = evidences().into_iter().map(Collection::Collected).collect();
        all.extend(FAILURES.iter().copied().map(Collection::Failed));
        all
    }

    /// An independently written truth table: the expected Assessment of every
    /// (requirement, evidence) pair over a small domain, stated as a table
    /// rather than derived from the function under test.
    #[test]
    fn assess_evidence_matches_truth_table() {
        let a = FileCondition::Absent;
        let any = FileCondition::Present {
            content: Content::Any,
        };
        let one = FileCondition::Present {
            content: Content::Exactly(digest(1)),
        };
        let table = [
            (a, FileEvidence::Absent, Assessment::Satisfied),
            (a, present(1), Assessment::Variance(Variance::Unexpected)),
            (
                any,
                FileEvidence::Absent,
                Assessment::Variance(Variance::Missing),
            ),
            (any, present(1), Assessment::Satisfied),
            (any, present(2), Assessment::Satisfied),
            (
                one,
                FileEvidence::Absent,
                Assessment::Variance(Variance::Missing),
            ),
            (one, present(1), Assessment::Satisfied),
            (
                one,
                present(2),
                Assessment::Variance(Variance::ContentDiffers {
                    expected: digest(1),
                    observed: digest(2),
                }),
            ),
        ];
        for (requirement, evidence, expected) in table {
            assert_eq!(
                assess_evidence(&requirement, &evidence),
                expected,
                "{requirement:?} against {evidence:?}"
            );
        }
    }

    #[test]
    fn assess_evidence_is_never_indeterminate() {
        for requirement in requirements() {
            for evidence in evidences() {
                assert!(!assess_evidence(&requirement, &evidence).is_indeterminate());
            }
        }
    }

    #[test]
    fn size_does_not_affect_content_judgment() {
        let requirement = FileCondition::Present {
            content: Content::Exactly(digest(1)),
        };
        for size in [0, 1, u64::MAX] {
            let evidence = FileEvidence::Present {
                digest: digest(1),
                size,
            };
            assert_eq!(
                assess_evidence(&requirement, &evidence),
                Assessment::Satisfied
            );
        }
    }

    /// N13: a failed collection is Indeterminate with its reason, never a
    /// Variance and never Satisfied, for every requirement.
    #[test]
    fn a_failed_collection_is_indeterminate() {
        for requirement in requirements() {
            for failure in FAILURES {
                assert_eq!(
                    assess_collection(&requirement, &Collection::Failed(failure)),
                    Assessment::Indeterminate(Reason::CollectionFailed(failure))
                );
            }
        }
    }

    #[test]
    fn a_collected_outcome_is_assessed_as_evidence() {
        for requirement in requirements() {
            for evidence in evidences() {
                assert_eq!(
                    assess_collection(&requirement, &Collection::Collected(evidence)),
                    assess_evidence(&requirement, &evidence)
                );
            }
        }
    }

    #[test]
    fn no_observation_is_indeterminate() {
        let condition = Condition::file(path("/etc/motd"), FileCondition::Absent);
        assert_eq!(
            assess(&condition, &[]),
            Assessment::Indeterminate(Reason::NoObservation)
        );
    }

    #[test]
    fn observations_of_other_resources_are_ignored() {
        let condition = Condition::file(path("/etc/motd"), FileCondition::Absent);
        let others = [
            observe("/etc/hosts", Collection::Collected(present(1))),
            observe("/etc/motd.d", Collection::Failed(CollectionFailure::Io)),
            observe("/etc", Collection::Collected(FileEvidence::Absent)),
        ];
        assert_eq!(
            assess(&condition, &others),
            Assessment::Indeterminate(Reason::NoObservation)
        );

        let mut with_own = others.to_vec();
        with_own.push(observe("/etc/motd", Collection::Collected(present(3))));
        assert_eq!(
            assess(&condition, &with_own),
            Assessment::Variance(Variance::Unexpected)
        );
    }

    /// Metamorphic relation: one Observation of the resource assesses exactly
    /// as its collection does.
    #[test]
    fn a_single_observation_assesses_as_its_collection() {
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/a"), requirement);
            for collection in collections() {
                assert_eq!(
                    assess(&condition, &[observe("/srv/a", collection)]),
                    assess_collection(&requirement, &collection)
                );
            }
        }
    }

    /// Metamorphic relation: repeating an Observation changes nothing.
    #[test]
    fn duplicate_observations_change_nothing() {
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/a"), requirement);
            for collection in collections() {
                let one = observe("/srv/a", collection);
                assert_eq!(
                    assess(&condition, &[one.clone(), one.clone(), one.clone()]),
                    assess(&condition, &[one])
                );
            }
        }
    }

    #[test]
    fn agreeing_collectors_are_sufficient() {
        let condition = Condition::file(
            path("/srv/a"),
            FileCondition::Present {
                content: Content::Exactly(digest(1)),
            },
        );
        let observations = [
            Observation::file(
                path("/srv/a"),
                Collection::Collected(present(1)),
                provenance("x", 0),
            ),
            Observation::file(
                path("/srv/a"),
                Collection::Collected(present(1)),
                provenance("y", 5),
            ),
        ];
        assert_eq!(assess(&condition, &observations), Assessment::Satisfied);
    }

    #[test]
    fn disagreeing_collected_observations_are_conflicting() {
        let cases = [
            (FileEvidence::Absent, present(1)),
            (present(1), present(2)),
            (
                present(1),
                FileEvidence::Present {
                    digest: digest(1),
                    size: 11,
                },
            ),
        ];
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/a"), requirement);
            for (left, right) in cases {
                for pair in [[left, right], [right, left]] {
                    let observations = [
                        observe("/srv/a", Collection::Collected(pair[0])),
                        observe("/srv/a", Collection::Collected(pair[1])),
                    ];
                    assert_eq!(
                        assess(&condition, &observations),
                        Assessment::Indeterminate(Reason::Conflicting)
                    );
                }
            }
        }
    }

    #[test]
    fn conflict_is_reported_even_alongside_failures() {
        let condition = Condition::file(path("/srv/a"), FileCondition::Absent);
        let observations = [
            observe(
                "/srv/a",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
            observe("/srv/a", Collection::Collected(FileEvidence::Absent)),
            observe("/srv/a", Collection::Collected(present(1))),
        ];
        assert_eq!(
            assess(&condition, &observations),
            Assessment::Indeterminate(Reason::Conflicting)
        );
    }

    #[test]
    fn all_failed_reports_the_smallest_failure_in_any_order() {
        let condition = Condition::file(
            path("/srv/a"),
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let forward = [
            observe("/srv/a", Collection::Failed(CollectionFailure::Io)),
            observe("/srv/a", Collection::Failed(CollectionFailure::TimedOut)),
            observe("/srv/a", Collection::Failed(CollectionFailure::Unsupported)),
        ];
        let mut backward = forward.clone();
        backward.reverse();
        let expected =
            Assessment::Indeterminate(Reason::CollectionFailed(CollectionFailure::TimedOut));
        assert_eq!(assess(&condition, &forward), expected);
        assert_eq!(assess(&condition, &backward), expected);
    }

    /// Reading of the `Reason` docs: `CollectionFailed` requires every
    /// Observation to have failed, so a failure beside collected evidence
    /// leaves the evidence to judge. Regression test for that reading.
    #[test]
    fn a_failure_beside_collected_evidence_does_not_hide_it() {
        let condition = Condition::file(path("/srv/a"), FileCondition::Absent);
        for failure in FAILURES {
            for observations in [
                [
                    observe("/srv/a", Collection::Failed(failure)),
                    observe("/srv/a", Collection::Collected(present(1))),
                ],
                [
                    observe("/srv/a", Collection::Collected(present(1))),
                    observe("/srv/a", Collection::Failed(failure)),
                ],
            ] {
                assert_eq!(
                    assess(&condition, &observations),
                    Assessment::Variance(Variance::Unexpected)
                );
            }
        }
    }

    /// Counterexample `partial-assessment`: an unobservable resource does
    /// not turn another resource's Variance into Indeterminate, and a
    /// Variance elsewhere does not hide an Indeterminate here.
    #[test]
    fn assessments_do_not_aggregate_across_resources() {
        let observations = [
            observe("/srv/a", Collection::Collected(FileEvidence::Absent)),
            observe(
                "/srv/b",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
        ];
        let a = Condition::file(
            path("/srv/a"),
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let b = Condition::file(path("/srv/b"), FileCondition::Absent);
        assert_eq!(
            assess(&a, &observations),
            Assessment::Variance(Variance::Missing)
        );
        assert_eq!(
            assess(&b, &observations),
            Assessment::Indeterminate(Reason::CollectionFailed(
                CollectionFailure::PermissionDenied
            ))
        );
    }

    /// N13 over `assess`: when no Observation of the resource was collected,
    /// the Assessment is Indeterminate for every requirement and every
    /// combination of failures, never Satisfied and never a Variance.
    #[test]
    fn only_failures_never_assess_as_satisfied_or_variance() {
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/a"), requirement);
            for first in FAILURES {
                for second in FAILURES {
                    let observations = [
                        observe("/srv/a", Collection::Failed(first)),
                        observe("/srv/a", Collection::Failed(second)),
                        observe("/srv/b", Collection::Collected(present(1))),
                    ];
                    assert_eq!(
                        assess(&condition, &observations),
                        Assessment::Indeterminate(Reason::CollectionFailed(first.min(second)))
                    );
                }
            }
        }
    }

    /// Metamorphic relation: the Assessment does not depend on the order of
    /// the Observations. Checked over every permutation of a mixed set.
    #[test]
    fn assessment_is_independent_of_observation_order() {
        let pool = [
            observe("/srv/a", Collection::Failed(CollectionFailure::Io)),
            observe(
                "/srv/a",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
            observe("/srv/a", Collection::Collected(present(2))),
            observe("/srv/b", Collection::Collected(present(1))),
        ];
        let sets: [&[usize]; 4] = [&[0, 1, 3], &[0, 1, 2, 3], &[0, 2, 3], &[1, 3]];
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/a"), requirement);
            for set in sets {
                let base: Vec<Observation> = set.iter().map(|&i| pool[i].clone()).collect();
                let expected = assess(&condition, &base);
                for order in permutations(set.len()) {
                    let permuted: Vec<Observation> =
                        order.iter().map(|&i| base[i].clone()).collect();
                    assert_eq!(assess(&condition, &permuted), expected);
                }
            }
        }
    }

    fn permutations(n: usize) -> Vec<Vec<usize>> {
        if n == 0 {
            return vec![Vec::new()];
        }
        let mut out = Vec::new();
        for rest in permutations(n - 1) {
            for slot in 0..=rest.len() {
                let mut next = rest.clone();
                next.insert(slot, n - 1);
                out.push(next);
            }
        }
        out
    }
}
