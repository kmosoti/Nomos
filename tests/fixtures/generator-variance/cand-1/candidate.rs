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
        (FileCondition::Present { content }, FileEvidence::Present { digest, .. }) => match content
        {
            Content::Any => Assessment::Satisfied,
            Content::Exactly(expected) => content_assessment(expected, digest),
        },
    }
}

/// Compares a required digest with an observed one.
fn content_assessment(expected: Digest, observed: Digest) -> Assessment {
    if expected == observed {
        Assessment::Satisfied
    } else {
        Assessment::Variance(Variance::ContentDiffers { expected, observed })
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
    // Split the Observations of this resource into collected evidence and
    // failures. Everything below is independent of arrival order: evidence
    // equality is an equivalence, and the failure is a minimum under a total
    // order.
    let mut evidence: Vec<FileEvidence> = Vec::new();
    let mut failures: Vec<CollectionFailure> = Vec::new();
    for observation in observations
        .iter()
        .filter(|observation| observation.path() == condition.path())
    {
        match observation.collection() {
            Collection::Collected(seen) => evidence.push(*seen),
            Collection::Failed(failure) => failures.push(*failure),
        }
    }

    match evidence.split_first() {
        // Collected evidence decides; a failure beside it is not evidence
        // and contradicts nothing. Evidence that disagrees is insufficient.
        Some((first, rest)) => {
            if rest.iter().all(|other| other == first) {
                assess_evidence(condition.requirement(), first)
            } else {
                Assessment::Indeterminate(Reason::Conflicting)
            }
        }
        None => match failures.iter().min() {
            Some(least) => Assessment::Indeterminate(Reason::CollectionFailed(*least)),
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
        Observation::file(path(at), collection, provenance("test", 0, 1))
    }

    fn present(byte: u8) -> FileEvidence {
        FileEvidence::Present {
            digest: digest(byte),
            size: 7,
        }
    }

    fn requirements() -> [FileCondition; 4] {
        [
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

    fn evidences() -> [FileEvidence; 4] {
        [
            FileEvidence::Absent,
            present(1),
            present(2),
            FileEvidence::Present {
                digest: digest(1),
                size: 0,
            },
        ]
    }

    /// An independently written oracle for `O |= C`, stated as a predicate
    /// rather than as the mapping `assess_evidence` computes.
    fn satisfies(requirement: &FileCondition, evidence: &FileEvidence) -> bool {
        match requirement {
            FileCondition::Absent => *evidence == FileEvidence::Absent,
            FileCondition::Present { content } => match evidence {
                FileEvidence::Absent => false,
                FileEvidence::Present { digest, .. } => match content {
                    Content::Any => true,
                    Content::Exactly(d) => d == digest,
                },
            },
        }
    }

    // Oracle: exhaustive truth table over the requirement and evidence cases,
    // written out by hand from the Variance docs.
    #[test]
    fn evidence_truth_table() {
        let d1 = digest(1);
        let d2 = digest(2);
        let any = FileCondition::Present {
            content: Content::Any,
        };
        let exactly1 = FileCondition::Present {
            content: Content::Exactly(d1),
        };
        let cases = [
            (
                FileCondition::Absent,
                FileEvidence::Absent,
                Assessment::Satisfied,
            ),
            (
                FileCondition::Absent,
                present(1),
                Assessment::Variance(Variance::Unexpected),
            ),
            (
                any,
                FileEvidence::Absent,
                Assessment::Variance(Variance::Missing),
            ),
            (any, present(1), Assessment::Satisfied),
            (any, present(2), Assessment::Satisfied),
            (
                exactly1,
                FileEvidence::Absent,
                Assessment::Variance(Variance::Missing),
            ),
            (exactly1, present(1), Assessment::Satisfied),
            (
                exactly1,
                present(2),
                Assessment::Variance(Variance::ContentDiffers {
                    expected: d1,
                    observed: d2,
                }),
            ),
        ];
        for (requirement, evidence, expected) in cases {
            assert_eq!(
                assess_evidence(&requirement, &evidence),
                expected,
                "{requirement:?} against {evidence:?}"
            );
        }
    }

    // Oracle: the soundness requirement of reconciliation.md. Collected
    // evidence is sufficient, so Satisfied iff O |= C, else a Variance,
    // never Indeterminate.
    #[test]
    fn evidence_is_satisfied_iff_it_satisfies_and_never_indeterminate() {
        for requirement in requirements() {
            for evidence in evidences() {
                let assessment = assess_evidence(&requirement, &evidence);
                assert!(!assessment.is_indeterminate());
                assert_eq!(
                    assessment == Assessment::Satisfied,
                    satisfies(&requirement, &evidence),
                    "{requirement:?} against {evidence:?}"
                );
                assert_eq!(
                    assessment.is_variance(),
                    !satisfies(&requirement, &evidence)
                );
            }
        }
    }

    // Oracle: N13. A failed collection is Indeterminate with its reason,
    // never Satisfied and never a Variance, for every requirement.
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

    // Oracle: the doc of assess_collection. Collected evidence is judged
    // exactly as assess_evidence judges it.
    #[test]
    fn collected_evidence_defers_to_assess_evidence() {
        for requirement in requirements() {
            for evidence in evidences() {
                assert_eq!(
                    assess_collection(&requirement, &Collection::Collected(evidence)),
                    assess_evidence(&requirement, &evidence)
                );
            }
        }
    }

    // Oracle: Reason::NoObservation's doc.
    #[test]
    fn no_observation_is_indeterminate() {
        let condition = Condition::file(path("/etc/motd"), FileCondition::Absent);
        assert_eq!(
            assess(&condition, &[]),
            Assessment::Indeterminate(Reason::NoObservation)
        );
    }

    // Oracle: assess's doc. Observations of other resources are ignored, so
    // they neither supply evidence nor count as an Observation.
    #[test]
    fn observations_of_other_resources_are_ignored() {
        let condition = Condition::file(path("/etc/motd"), FileCondition::Absent);
        let others = vec![
            observe("/etc/motd.d", Collection::Collected(present(1))),
            observe("/etc", Collection::Failed(CollectionFailure::Io)),
        ];
        assert_eq!(
            assess(&condition, &others),
            Assessment::Indeterminate(Reason::NoObservation)
        );

        let mut mixed = others.clone();
        mixed.push(observe(
            "/etc/motd",
            Collection::Collected(FileEvidence::Absent),
        ));
        assert_eq!(assess(&condition, &mixed), Assessment::Satisfied);
    }

    // Oracle: a single Observation of the resource is judged as its
    // collection is judged (metamorphic relation between assess and
    // assess_collection).
    #[test]
    fn one_observation_agrees_with_assess_collection() {
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/a"), requirement);
            let mut collections: Vec<Collection> =
                evidences().into_iter().map(Collection::Collected).collect();
            collections.extend(FAILURES.into_iter().map(Collection::Failed));
            for collection in collections {
                assert_eq!(
                    assess(&condition, &[observe("/srv/a", collection)]),
                    assess_collection(&requirement, &collection)
                );
            }
        }
    }

    // Oracle: Reason::CollectionFailed's doc. When every Observation fails,
    // the reason is the smallest failure, whatever the arrival order.
    #[test]
    fn every_failure_reports_the_smallest_reason_in_any_order() {
        let condition = Condition::file(
            path("/srv/a"),
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let orders = [[3usize, 1, 2], [2, 3, 1], [1, 2, 3], [3, 2, 1]];
        for order in orders {
            let observations: Vec<Observation> = order
                .iter()
                .map(|&i| observe("/srv/a", Collection::Failed(FAILURES[i])))
                .collect();
            assert_eq!(
                assess(&condition, &observations),
                Assessment::Indeterminate(Reason::CollectionFailed(CollectionFailure::TimedOut))
            );
        }
    }

    // Oracle: the soundness requirement. Collected Observations that
    // contradict one another are not sufficient evidence, so the result is
    // Indeterminate, never Satisfied or a Variance, in either order.
    #[test]
    fn conflicting_evidence_is_indeterminate() {
        let condition = Condition::file(
            path("/srv/a"),
            FileCondition::Present {
                content: Content::Exactly(digest(1)),
            },
        );
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
        for (a, b) in pairs {
            for (x, y) in [(a, b), (b, a)] {
                let observations = vec![
                    observe("/srv/a", Collection::Collected(x)),
                    observe("/srv/a", Collection::Collected(y)),
                ];
                assert_eq!(
                    assess(&condition, &observations),
                    Assessment::Indeterminate(Reason::Conflicting)
                );
            }
        }
    }

    // Oracle: the soundness requirement. A conflict is not hidden by a
    // failure or by a third Observation that agrees with one side.
    #[test]
    fn conflict_wins_over_failures_and_agreement() {
        let condition = Condition::file(path("/srv/a"), FileCondition::Absent);
        let observations = vec![
            observe("/srv/a", Collection::Collected(FileEvidence::Absent)),
            observe(
                "/srv/a",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
            observe("/srv/a", Collection::Collected(FileEvidence::Absent)),
            observe("/srv/a", Collection::Collected(present(3))),
        ];
        assert_eq!(
            assess(&condition, &observations),
            Assessment::Indeterminate(Reason::Conflicting)
        );
    }

    // Oracle: the soundness requirement. Agreeing collected Observations,
    // from different collectors and windows, are sufficient evidence.
    #[test]
    fn agreeing_evidence_is_judged() {
        let condition = Condition::file(
            path("/srv/a"),
            FileCondition::Present {
                content: Content::Exactly(digest(1)),
            },
        );
        let observations = vec![
            Observation::file(
                path("/srv/a"),
                Collection::Collected(present(2)),
                provenance("one", 0, 5),
            ),
            Observation::file(
                path("/srv/a"),
                Collection::Collected(present(2)),
                provenance("two", 10, 10),
            ),
        ];
        assert_eq!(
            assess(&condition, &observations),
            Assessment::Variance(Variance::ContentDiffers {
                expected: digest(1),
                observed: digest(2),
            })
        );
    }

    // Interpretation, not a stated oracle: a failed collection is not
    // evidence and so contradicts nothing. When another Observation of the
    // resource was collected, that one decides. Regression test for this
    // reading of Reason::CollectionFailed ("every Observation ... failed").
    #[test]
    fn a_failure_beside_collected_evidence_does_not_decide() {
        let condition = Condition::file(path("/srv/a"), FileCondition::Absent);
        for failure in FAILURES {
            for observations in [
                vec![
                    observe("/srv/a", Collection::Failed(failure)),
                    observe("/srv/a", Collection::Collected(present(1))),
                ],
                vec![
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

    // Oracle: "Assessments do not aggregate" and the `partial-assessment`
    // counterexample. An Indeterminate resource does not erase a Variance
    // of another, and a Variance does not become Indeterminate because an
    // unrelated resource could not be observed.
    #[test]
    fn assessments_do_not_aggregate() {
        let observations = vec![
            observe(
                "/srv/denied",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
            observe("/srv/drifted", Collection::Collected(present(2))),
        ];
        let denied = Condition::file(
            path("/srv/denied"),
            FileCondition::Present {
                content: Content::Exactly(digest(1)),
            },
        );
        let drifted = Condition::file(
            path("/srv/drifted"),
            FileCondition::Present {
                content: Content::Exactly(digest(1)),
            },
        );
        assert_eq!(
            assess(&denied, &observations),
            Assessment::Indeterminate(Reason::CollectionFailed(
                CollectionFailure::PermissionDenied
            ))
        );
        assert_eq!(
            assess(&drifted, &observations),
            Assessment::Variance(Variance::ContentDiffers {
                expected: digest(1),
                observed: digest(2),
            })
        );
    }

    // Oracle: rule 7 (determinism) as a metamorphic relation. The result is
    // invariant under every permutation of a mixed set of Observations.
    #[test]
    fn assess_is_invariant_under_permutation() {
        let pool = [
            observe("/srv/a", Collection::Failed(CollectionFailure::Io)),
            observe("/srv/a", Collection::Failed(CollectionFailure::TimedOut)),
            observe("/srv/a", Collection::Collected(present(1))),
            observe("/srv/a", Collection::Collected(present(1))),
            observe("/srv/b", Collection::Collected(present(2))),
        ];
        let conflicting_extra = observe("/srv/a", Collection::Collected(FileEvidence::Absent));
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/a"), requirement);
            for with_conflict in [false, true] {
                let mut items: Vec<Observation> = pool.to_vec();
                if with_conflict {
                    items.push(conflicting_extra.clone());
                }
                let reference = assess(&condition, &items);
                let mut indices: Vec<usize> = (0..items.len()).collect();
                for_each_permutation(&mut indices, 0, &mut |perm| {
                    let permuted: Vec<Observation> =
                        perm.iter().map(|&i| items[i].clone()).collect();
                    assert_eq!(assess(&condition, &permuted), reference);
                });
            }
        }
    }

    fn for_each_permutation(items: &mut [usize], k: usize, f: &mut dyn FnMut(&[usize])) {
        if k == items.len() {
            f(items);
            return;
        }
        for i in k..items.len() {
            items.swap(k, i);
            for_each_permutation(items, k + 1, f);
            items.swap(k, i);
        }
    }
}
