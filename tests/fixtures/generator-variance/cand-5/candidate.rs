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
            assess_content(content, *digest)
        }
    }
}

/// Judges the digest of a present file against a content requirement.
fn assess_content(content: &Content, observed: Digest) -> Assessment {
    match content {
        Content::Any => Assessment::Satisfied,
        Content::Exactly(expected) if *expected == observed => Assessment::Satisfied,
        Content::Exactly(expected) => Assessment::Variance(Variance::ContentDiffers {
            expected: *expected,
            observed,
        }),
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
    let mut collected: Vec<FileEvidence> = Vec::new();
    let mut least_failure: Option<CollectionFailure> = None;

    for observation in observations
        .iter()
        .filter(|observation| observation.path() == condition.path())
    {
        match observation.collection() {
            Collection::Collected(evidence) => collected.push(*evidence),
            Collection::Failed(failure) => {
                least_failure = Some(match least_failure {
                    Some(least) if least <= *failure => least,
                    _ => *failure,
                });
            }
        }
    }

    // Collected evidence decides when there is any: a failed collection
    // contradicts nothing, it only fails to add evidence. Collected evidence
    // that disagrees with other collected evidence is not sufficient.
    if let Some((first, rest)) = collected.split_first() {
        if rest.iter().any(|evidence| evidence != first) {
            return Assessment::Indeterminate(Reason::Conflicting);
        }
        return assess_evidence(condition.requirement(), first);
    }

    match least_failure {
        Some(failure) => Assessment::Indeterminate(Reason::CollectionFailed(failure)),
        None => Assessment::Indeterminate(Reason::NoObservation),
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

    fn provenance(collector: &str) -> Provenance {
        Provenance::new(
            CollectorId::new(collector).unwrap(),
            Window::new(Instant(10), Instant(20)).unwrap(),
        )
    }

    fn observation(at: &str, collection: Collection) -> Observation {
        Observation::file(path(at), collection, provenance("test-collector"))
    }

    fn present(byte: u8, size: u64) -> FileEvidence {
        FileEvidence::Present {
            digest: digest(byte),
            size,
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
        ]
    }

    fn evidences() -> Vec<FileEvidence> {
        vec![FileEvidence::Absent, present(1, 5), present(2, 5)]
    }

    /// Every permutation of `items`, by Heap's algorithm.
    fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
        fn heap<T: Clone>(k: usize, items: &mut Vec<T>, out: &mut Vec<Vec<T>>) {
            if k <= 1 {
                out.push(items.clone());
                return;
            }
            for i in 0..k {
                heap(k - 1, items, out);
                if k.is_multiple_of(2) {
                    items.swap(i, k - 1);
                } else {
                    items.swap(0, k - 1);
                }
            }
        }
        let mut items = items.to_vec();
        let mut out = Vec::new();
        heap(items.len(), &mut items, &mut out);
        out
    }

    // Oracle: an exhaustive truth table written from the documentation of
    // `Variance`, `FileCondition`, and `Content`, not from the code.
    #[test]
    fn evidence_truth_table() {
        let d1 = digest(1);
        let d2 = digest(2);
        let exactly_d1 = FileCondition::Present {
            content: Content::Exactly(d1),
        };
        let any = FileCondition::Present {
            content: Content::Any,
        };
        let table = [
            (
                FileCondition::Absent,
                FileEvidence::Absent,
                Assessment::Satisfied,
            ),
            (
                FileCondition::Absent,
                present(1, 5),
                Assessment::Variance(Variance::Unexpected),
            ),
            (
                any,
                FileEvidence::Absent,
                Assessment::Variance(Variance::Missing),
            ),
            (any, present(1, 5), Assessment::Satisfied),
            (any, present(2, 0), Assessment::Satisfied),
            (
                exactly_d1,
                FileEvidence::Absent,
                Assessment::Variance(Variance::Missing),
            ),
            (exactly_d1, present(1, 5), Assessment::Satisfied),
            (
                exactly_d1,
                present(2, 5),
                Assessment::Variance(Variance::ContentDiffers {
                    expected: d1,
                    observed: d2,
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

    // Oracle: the doc contract of `assess_evidence` (never Indeterminate).
    #[test]
    fn evidence_is_never_indeterminate() {
        for requirement in requirements() {
            for evidence in evidences() {
                assert!(!assess_evidence(&requirement, &evidence).is_indeterminate());
            }
        }
    }

    // Oracle: invariant N13. A failed collection is Indeterminate with its
    // reason, never Satisfied and never a Variance, for every requirement.
    #[test]
    fn a_failed_collection_is_indeterminate() {
        for requirement in requirements() {
            for failure in FAILURES {
                let assessment = assess_collection(&requirement, &Collection::Failed(failure));
                assert_eq!(
                    assessment,
                    Assessment::Indeterminate(Reason::CollectionFailed(failure))
                );
                assert!(!assessment.is_variance());
            }
        }
    }

    // Oracle: the doc contract of `assess_collection` (evidence delegates).
    #[test]
    fn collected_evidence_is_judged_as_evidence() {
        for requirement in requirements() {
            for evidence in evidences() {
                assert_eq!(
                    assess_collection(&requirement, &Collection::Collected(evidence)),
                    assess_evidence(&requirement, &evidence)
                );
            }
        }
    }

    // Oracle: `Reason::NoObservation` documentation.
    #[test]
    fn no_observation_is_indeterminate() {
        for requirement in requirements() {
            let condition = Condition::file(path("/etc/a"), requirement);
            assert_eq!(
                assess(&condition, &[]),
                Assessment::Indeterminate(Reason::NoObservation)
            );
        }
    }

    // Oracle: the doc contract of `assess` (other resources are ignored).
    #[test]
    fn observations_of_other_resources_are_ignored() {
        let condition = Condition::file(path("/etc/a"), FileCondition::Absent);
        let others = [
            observation("/etc/b", Collection::Collected(present(1, 5))),
            observation("/etc/a/b", Collection::Failed(CollectionFailure::Io)),
            observation("/etc", Collection::Collected(FileEvidence::Absent)),
        ];
        assert_eq!(
            assess(&condition, &others),
            Assessment::Indeterminate(Reason::NoObservation)
        );

        let mut with_own = others.to_vec();
        with_own.push(observation(
            "/etc/a",
            Collection::Collected(FileEvidence::Absent),
        ));
        assert_eq!(assess(&condition, &with_own), Assessment::Satisfied);
    }

    // Oracle: reconciliation.md, "Assessments do not aggregate" (counterexample
    // `partial-assessment`). A failure elsewhere does not turn a Variance here
    // into Indeterminate.
    #[test]
    fn a_failure_elsewhere_does_not_hide_a_variance() {
        let condition = Condition::file(
            path("/etc/a"),
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let observations = [
            observation(
                "/etc/b",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
            observation("/etc/a", Collection::Collected(FileEvidence::Absent)),
        ];
        assert_eq!(
            assess(&condition, &observations),
            Assessment::Variance(Variance::Missing)
        );
    }

    // Oracle: a single Observation of the resource reduces to
    // `assess_collection` (metamorphic relation between the two functions).
    #[test]
    fn one_observation_agrees_with_assess_collection() {
        let mut collections: Vec<Collection> =
            evidences().into_iter().map(Collection::Collected).collect();
        collections.extend(FAILURES.iter().copied().map(Collection::Failed));
        for requirement in requirements() {
            let condition = Condition::file(path("/srv/x"), requirement);
            for collection in &collections {
                assert_eq!(
                    assess(&condition, &[observation("/srv/x", *collection)]),
                    assess_collection(&requirement, collection)
                );
            }
        }
    }

    // Oracle: `Reason::CollectionFailed` documentation (the smallest failure
    // under `CollectionFailure`'s ordering, whatever the arrival order).
    #[test]
    fn every_failure_reports_the_least_reason_in_any_order() {
        let condition = Condition::file(path("/etc/a"), FileCondition::Absent);
        let failures = [
            CollectionFailure::Io,
            CollectionFailure::TimedOut,
            CollectionFailure::Unsupported,
        ];
        for order in permutations(&failures) {
            let observations: Vec<Observation> = order
                .iter()
                .map(|failure| observation("/etc/a", Collection::Failed(*failure)))
                .collect();
            assert_eq!(
                assess(&condition, &observations),
                Assessment::Indeterminate(Reason::CollectionFailed(CollectionFailure::TimedOut))
            );
        }
        let all: Vec<Observation> = FAILURES
            .iter()
            .rev()
            .map(|failure| observation("/etc/a", Collection::Failed(*failure)))
            .collect();
        assert_eq!(
            assess(&condition, &all),
            Assessment::Indeterminate(Reason::CollectionFailed(
                CollectionFailure::PermissionDenied
            ))
        );
    }

    // Oracle: reconciliation.md, "sufficient" excludes Observations that
    // contradict another Observation of r. Disagreement is Indeterminate even
    // when each piece of evidence alone would satisfy the Condition.
    #[test]
    fn disagreeing_evidence_is_conflicting() {
        let any = Condition::file(
            path("/etc/a"),
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let absent = Condition::file(path("/etc/a"), FileCondition::Absent);
        let cases = [
            [present(1, 5), present(2, 5)],
            [present(1, 5), present(1, 6)],
            [present(1, 5), FileEvidence::Absent],
        ];
        for condition in [&any, &absent] {
            for pair in cases {
                let observations: Vec<Observation> = pair
                    .iter()
                    .map(|evidence| observation("/etc/a", Collection::Collected(*evidence)))
                    .collect();
                assert_eq!(
                    assess(condition, &observations),
                    Assessment::Indeterminate(Reason::Conflicting)
                );
            }
        }
    }

    // Oracle: reconciliation.md soundness requirement; agreeing collected
    // Observations are sufficient evidence, judged as that evidence.
    #[test]
    fn agreeing_evidence_is_judged_once() {
        for requirement in requirements() {
            let condition = Condition::file(path("/etc/a"), requirement);
            for evidence in evidences() {
                let observations = [
                    observation("/etc/a", Collection::Collected(evidence)),
                    Observation::file(
                        path("/etc/a"),
                        Collection::Collected(evidence),
                        provenance("another-collector"),
                    ),
                ];
                assert_eq!(
                    assess(&condition, &observations),
                    assess_evidence(&requirement, &evidence)
                );
            }
        }
    }

    // Interpretive choice (see NOTES.md): a failed collection contradicts
    // nothing, so collected evidence decides when some Observations failed.
    // Regression test for that choice; its oracle is the reading of
    // `Reason::CollectionFailed` ("every Observation of the resource failed").
    #[test]
    fn collected_evidence_outweighs_a_failed_collection() {
        let condition = Condition::file(
            path("/etc/a"),
            FileCondition::Present {
                content: Content::Exactly(digest(1)),
            },
        );
        let observations = [
            observation(
                "/etc/a",
                Collection::Failed(CollectionFailure::PermissionDenied),
            ),
            observation("/etc/a", Collection::Collected(present(2, 5))),
        ];
        for order in permutations(&observations) {
            assert_eq!(
                assess(&condition, &order),
                Assessment::Variance(Variance::ContentDiffers {
                    expected: digest(1),
                    observed: digest(2),
                })
            );
        }
    }

    // Oracle: metamorphic relation. The Assessment does not depend on the
    // order the Observations arrived in (rule 7, `Reason` documentation).
    #[test]
    fn assessment_is_invariant_under_permutation() {
        let pool = [
            observation("/etc/a", Collection::Collected(present(1, 5))),
            observation("/etc/a", Collection::Failed(CollectionFailure::Unsupported)),
            observation("/etc/a", Collection::Failed(CollectionFailure::TimedOut)),
            observation("/etc/a", Collection::Collected(present(2, 5))),
            observation("/etc/b", Collection::Collected(FileEvidence::Absent)),
        ];
        // Every subset of the pool, in every order.
        for mask in 0u32..(1 << pool.len()) {
            let subset: Vec<Observation> = pool
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, o)| o.clone())
                .collect();
            for requirement in requirements() {
                let condition = Condition::file(path("/etc/a"), requirement);
                let reference = assess(&condition, &subset);
                for order in permutations(&subset) {
                    assert_eq!(assess(&condition, &order), reference);
                }
            }
        }
    }

    // Oracle: invariant N13 on generated inputs. When no Observation of the
    // resource was collected, the Assessment is Indeterminate, never a
    // Variance and never Satisfied.
    #[test]
    fn without_collected_evidence_nothing_is_known() {
        let mut observations: Vec<Observation> = Vec::new();
        for failure in FAILURES {
            observations.push(observation("/etc/a", Collection::Failed(failure)));
            observations.push(observation("/etc/b", Collection::Collected(present(9, 1))));
            for requirement in requirements() {
                let condition = Condition::file(path("/etc/a"), requirement);
                let assessment = assess(&condition, &observations);
                assert!(assessment.is_indeterminate(), "{assessment:?}");
                assert_ne!(assessment, Assessment::Satisfied);
            }
        }
    }
}
