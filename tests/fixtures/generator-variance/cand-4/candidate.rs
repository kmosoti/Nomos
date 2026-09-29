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
        (FileCondition::Present { content }, FileEvidence::Present { digest, .. }) => {
            assess_content(content, digest)
        }
    }
}

/// Judges the content of a file known to exist against a content requirement.
fn assess_content(content: &Content, observed: &Digest) -> Assessment {
    match content {
        Content::Any => Assessment::Satisfied,
        Content::Exactly(expected) if expected == observed => Assessment::Satisfied,
        Content::Exactly(expected) => Assessment::Variance(Variance::ContentDiffers {
            expected: *expected,
            observed: *observed,
        }),
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
    let relevant = observations
        .iter()
        .filter(|observation| observation.path() == condition.path());

    // Split the Observations of this resource into the evidence collected and
    // the smallest failure seen. Both are independent of arrival order: the
    // evidence is only ever compared for equality, and the failure is a
    // minimum.
    let mut collected: Vec<FileEvidence> = Vec::new();
    let mut smallest_failure: Option<CollectionFailure> = None;
    for observation in relevant {
        match observation.collection() {
            Collection::Collected(evidence) => collected.push(*evidence),
            Collection::Failed(failure) => {
                smallest_failure = Some(match smallest_failure {
                    Some(current) if current <= *failure => current,
                    _ => *failure,
                });
            }
        }
    }

    match collected.split_first() {
        // At least one Observation collected evidence. A failed Observation
        // alongside it adds nothing and contradicts nothing; only collected
        // Observations that disagree make the evidence insufficient.
        Some((first, rest)) => {
            if rest.iter().all(|evidence| evidence == first) {
                assess_evidence(condition.requirement(), first)
            } else {
                Assessment::Indeterminate(Reason::Conflicting)
            }
        }
        None => match smallest_failure {
            Some(failure) => Assessment::Indeterminate(Reason::CollectionFailed(failure)),
            None => Assessment::Indeterminate(Reason::NoObservation),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{CollectorId, Instant, Provenance, Window};
    use crate::resource::ResourcePath;
    use alloc::vec;

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

    fn provenance(collector: &str, start: u64, end: u64) -> Provenance {
        Provenance::new(
            CollectorId::new(collector).unwrap(),
            Window::new(Instant(start), Instant(end)).unwrap(),
        )
    }

    fn observe(at: &str, collection: Collection) -> Observation {
        Observation::file(path(at), collection, provenance("test", 1, 2))
    }

    fn present(byte: u8) -> FileEvidence {
        FileEvidence::Present {
            digest: digest(byte),
            size: 7,
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
        vec![
            FileEvidence::Absent,
            present(1),
            present(2),
            FileEvidence::Present {
                digest: digest(1),
                size: 0,
            },
        ]
    }

    fn collections() -> Vec<Collection> {
        let mut all: Vec<Collection> = evidences().into_iter().map(Collection::Collected).collect();
        all.extend(FAILURES.iter().copied().map(Collection::Failed));
        all
    }

    /// Independent reference for `assess_evidence`, written as the truth
    /// table of the Condition semantics rather than as a match on pairs.
    fn reference(requirement: &FileCondition, evidence: &FileEvidence) -> Assessment {
        let exists = matches!(evidence, FileEvidence::Present { .. });
        let wants_file = matches!(requirement, FileCondition::Present { .. });
        if wants_file && !exists {
            return Assessment::Variance(Variance::Missing);
        }
        if !wants_file && exists {
            return Assessment::Variance(Variance::Unexpected);
        }
        if let (
            FileCondition::Present {
                content: Content::Exactly(expected),
            },
            FileEvidence::Present { digest, .. },
        ) = (requirement, evidence)
        {
            if expected != digest {
                return Assessment::Variance(Variance::ContentDiffers {
                    expected: *expected,
                    observed: *digest,
                });
            }
        }
        Assessment::Satisfied
    }

    // Oracle: exhaustive truth table over a representative domain, against
    // an independently written reference.
    #[test]
    fn assess_evidence_matches_the_truth_table() {
        for requirement in requirements() {
            for evidence in evidences() {
                assert_eq!(
                    assess_evidence(&requirement, &evidence),
                    reference(&requirement, &evidence),
                    "{requirement:?} against {evidence:?}"
                );
            }
        }
    }

    // Oracle: the docstring's postcondition (never Indeterminate).
    #[test]
    fn assess_evidence_is_never_indeterminate() {
        for requirement in requirements() {
            for evidence in evidences() {
                assert!(!assess_evidence(&requirement, &evidence).is_indeterminate());
            }
        }
    }

    // Regression tests naming each Variance directly.
    #[test]
    fn assess_evidence_names_each_variance() {
        let absent = FileCondition::Absent;
        let exactly_one = FileCondition::Present {
            content: Content::Exactly(digest(1)),
        };
        assert_eq!(
            assess_evidence(&absent, &present(1)),
            Assessment::Variance(Variance::Unexpected)
        );
        assert_eq!(
            assess_evidence(&exactly_one, &FileEvidence::Absent),
            Assessment::Variance(Variance::Missing)
        );
        assert_eq!(
            assess_evidence(&exactly_one, &present(2)),
            Assessment::Variance(Variance::ContentDiffers {
                expected: digest(1),
                observed: digest(2),
            })
        );
        assert_eq!(
            assess_evidence(&exactly_one, &present(1)),
            Assessment::Satisfied
        );
        assert_eq!(
            assess_evidence(
                &FileCondition::Present {
                    content: Content::Any
                },
                &present(9)
            ),
            Assessment::Satisfied
        );
        assert_eq!(
            assess_evidence(&absent, &FileEvidence::Absent),
            Assessment::Satisfied
        );
    }

    // Oracle: invariant N13. A failed collection is Indeterminate with its
    // reason, for every requirement, and never Satisfied or a Variance.
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

    // Oracle: the docstring's delegation law.
    #[test]
    fn a_collected_collection_is_judged_on_its_evidence() {
        for requirement in requirements() {
            for evidence in evidences() {
                assert_eq!(
                    assess_collection(&requirement, &Collection::Collected(evidence)),
                    assess_evidence(&requirement, &evidence)
                );
            }
        }
    }

    // Oracle: the soundness requirement's "denied read" clause.
    #[test]
    fn a_denied_read_never_assesses_as_satisfied_or_variance() {
        for requirement in requirements() {
            let condition = Condition::file(path("/etc/motd"), requirement);
            let denied = observe(
                "/etc/motd",
                Collection::Failed(CollectionFailure::PermissionDenied),
            );
            let result = assess(&condition, &[denied]);
            assert!(result.is_indeterminate());
            assert!(!result.is_variance());
            assert_ne!(result, Assessment::Satisfied);
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

    // Oracle: the function's contract; an Absent Observation of another path
    // must not satisfy an Absent Condition here.
    #[test]
    fn observations_of_other_resources_are_ignored() {
        let condition = Condition::file(path("/etc/motd"), FileCondition::Absent);
        let elsewhere = [
            observe("/etc/motd2", Collection::Collected(FileEvidence::Absent)),
            observe("/etc", Collection::Collected(FileEvidence::Absent)),
            observe("/etc/motd/x", Collection::Failed(CollectionFailure::Io)),
        ];
        assert_eq!(
            assess(&condition, &elsewhere),
            Assessment::Indeterminate(Reason::NoObservation)
        );

        let mut with_own = elsewhere.to_vec();
        with_own.push(observe("/etc/motd", Collection::Collected(present(1))));
        assert_eq!(
            assess(&condition, &with_own),
            Assessment::Variance(Variance::Unexpected)
        );
    }

    // Oracle: metamorphic relation. A single Observation of the resource is
    // assessed exactly as its collection is.
    #[test]
    fn one_observation_agrees_with_assess_collection() {
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

    // Oracle: `Reason::CollectionFailed` docs; the smallest failure, whatever
    // the arrival order.
    #[test]
    fn every_failure_reports_the_smallest_reason_in_any_order() {
        let condition = Condition::file(path("/srv/a"), FileCondition::Absent);
        let orders: [[usize; 4]; 4] = [[0, 1, 2, 3], [3, 2, 1, 0], [1, 3, 0, 2], [2, 0, 3, 1]];
        for order in orders {
            let observations: Vec<Observation> = order
                .iter()
                .map(|&i| observe("/srv/a", Collection::Failed(FAILURES[i])))
                .collect();
            assert_eq!(
                assess(&condition, &observations),
                Assessment::Indeterminate(Reason::CollectionFailed(
                    CollectionFailure::PermissionDenied
                ))
            );
        }
        let later = [
            observe("/srv/a", Collection::Failed(CollectionFailure::Io)),
            observe("/srv/a", Collection::Failed(CollectionFailure::TimedOut)),
        ];
        assert_eq!(
            assess(&condition, &later),
            Assessment::Indeterminate(Reason::CollectionFailed(CollectionFailure::TimedOut))
        );
    }

    // Oracle: the soundness requirement; evidence contradicting another
    // Observation of the resource is insufficient.
    #[test]
    fn disagreeing_collected_observations_are_conflicting() {
        let pairs = [
            (FileEvidence::Absent, present(1)),
            (present(1), present(2)),
            (
                present(1),
                FileEvidence::Present {
                    digest: digest(1),
                    size: 0,
                },
            ),
        ];
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/a"), requirement);
            for (a, b) in pairs {
                for observations in [
                    [
                        observe("/srv/a", Collection::Collected(a)),
                        observe("/srv/a", Collection::Collected(b)),
                    ],
                    [
                        observe("/srv/a", Collection::Collected(b)),
                        observe("/srv/a", Collection::Collected(a)),
                    ],
                ] {
                    assert_eq!(
                        assess(&condition, &observations),
                        Assessment::Indeterminate(Reason::Conflicting)
                    );
                }
            }
        }
    }

    // Conflict wins over failures: a failure does not hide a contradiction.
    #[test]
    fn conflict_is_reported_even_alongside_failures() {
        let condition = Condition::file(path("/srv/a"), FileCondition::Absent);
        let observations = [
            observe(
                "/srv/a",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
            observe("/srv/a", Collection::Collected(present(1))),
            observe("/srv/a", Collection::Collected(FileEvidence::Absent)),
        ];
        assert_eq!(
            assess(&condition, &observations),
            Assessment::Indeterminate(Reason::Conflicting)
        );
    }

    // Agreeing Observations from different collectors and windows are
    // sufficient; duplicates do not change the Assessment.
    #[test]
    fn agreeing_observations_are_assessed() {
        let requirement = FileCondition::Present {
            content: Content::Exactly(digest(1)),
        };
        let condition = Condition::file(path("/srv/a"), requirement);
        let agreeing = [
            Observation::file(
                path("/srv/a"),
                Collection::Collected(present(2)),
                provenance("one", 1, 5),
            ),
            Observation::file(
                path("/srv/a"),
                Collection::Collected(present(2)),
                provenance("two", 3, 3),
            ),
        ];
        assert_eq!(
            assess(&condition, &agreeing),
            Assessment::Variance(Variance::ContentDiffers {
                expected: digest(1),
                observed: digest(2),
            })
        );
    }

    // A failed Observation beside collected evidence neither erases nor
    // contradicts it. This is an interpretive choice; see NOTES.md.
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

    // Oracle: counterexample `partial-assessment`. Assessments do not
    // aggregate: an unobservable resource leaves another's Variance intact.
    #[test]
    fn an_indeterminate_resource_does_not_erase_another_variance() {
        let missing = Condition::file(
            path("/srv/a"),
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let unreadable = Condition::file(path("/srv/b"), FileCondition::Absent);
        let observations = [
            observe("/srv/a", Collection::Collected(FileEvidence::Absent)),
            observe(
                "/srv/b",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
        ];
        assert_eq!(
            assess(&missing, &observations),
            Assessment::Variance(Variance::Missing)
        );
        assert_eq!(
            assess(&unreadable, &observations),
            Assessment::Indeterminate(Reason::CollectionFailed(
                CollectionFailure::PermissionDenied
            ))
        );
    }

    // Oracle: metamorphic relation. Reversing the Observations never changes
    // the Assessment (rule 7, determinism), over every pair and triple of a
    // small collection domain.
    #[test]
    fn assessment_does_not_depend_on_arrival_order() {
        let domain = collections();
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/a"), requirement);
            for a in &domain {
                for b in &domain {
                    for c in &domain {
                        let forward = [
                            observe("/srv/a", *a),
                            observe("/srv/a", *b),
                            observe("/srv/a", *c),
                        ];
                        let mut backward = forward.clone();
                        backward.reverse();
                        let mut rotated = forward.clone();
                        rotated.rotate_left(1);
                        let expected = assess(&condition, &forward);
                        assert_eq!(assess(&condition, &backward), expected);
                        assert_eq!(assess(&condition, &rotated), expected);
                    }
                }
            }
        }
    }
}
