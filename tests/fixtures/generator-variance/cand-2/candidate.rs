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
            judge_content(content, digest)
        }
    }
}

/// Judges the digest of a present file against a content requirement.
fn judge_content(content: &Content, observed: &Digest) -> Assessment {
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

    // Evidence that was actually collected. Failed collections carry no
    // evidence, so they cannot contradict what another collector saw.
    let mut collected = relevant.iter().filter_map(|collection| match collection {
        Collection::Collected(evidence) => Some(evidence),
        Collection::Failed(_) => None,
    });

    match collected.next() {
        Some(first) => {
            if collected.all(|evidence| evidence == first) {
                assess_evidence(condition.requirement(), first)
            } else {
                Assessment::Indeterminate(Reason::Conflicting)
            }
        }
        None => {
            // Nothing was collected: either nothing was observed, or every
            // Observation failed. The smallest failure keeps the result
            // independent of arrival order.
            let smallest = relevant
                .iter()
                .filter_map(|collection| match collection {
                    Collection::Failed(failure) => Some(*failure),
                    Collection::Collected(_) => None,
                })
                .min();
            match smallest {
                Some(failure) => Assessment::Indeterminate(Reason::CollectionFailed(failure)),
                None => Assessment::Indeterminate(Reason::NoObservation),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{CollectorId, Instant, Provenance, Window};
    use crate::resource::ResourcePath;
    use alloc::vec;

    // Oracle notes. `evidence_truth_table` is an exhaustive truth table
    // written from the definitions in the pack (spec §8, reconciliation.md
    // soundness requirement, the `Variance` docs), not from the code. The
    // N13 tests check the invariant "a failed collection is never Satisfied
    // or a Variance" over every requirement and failure. The permutation
    // tests check a metamorphic relation: `assess` is invariant under the
    // order of its Observations (the `Reason::CollectionFailed` docs). The
    // remaining `assess` tests are regression tests for the interpretation
    // recorded in each test's comment.

    const FAILURES: [CollectionFailure; 4] = [
        CollectionFailure::PermissionDenied,
        CollectionFailure::TimedOut,
        CollectionFailure::Unsupported,
        CollectionFailure::Io,
    ];

    fn digest(byte: u8) -> Digest {
        Digest::from_bytes([byte; 32])
    }

    fn present(byte: u8) -> FileEvidence {
        FileEvidence::Present {
            digest: digest(byte),
            size: u64::from(byte),
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

    fn path(text: &str) -> ResourcePath {
        match ResourcePath::new(text) {
            Ok(path) => path,
            Err(error) => panic!("test path {text:?} is invalid: {error}"),
        }
    }

    fn provenance(name: &str, at: u64) -> Provenance {
        let collector = CollectorId::new(name).expect("non-empty collector name");
        let window = Window::new(Instant(at), Instant(at)).expect("valid window");
        Provenance::new(collector, window)
    }

    fn observe(text: &str, collection: Collection) -> Observation {
        Observation::file(path(text), collection, provenance("test", 0))
    }

    fn condition(requirement: FileCondition) -> Condition {
        Condition::file(path("/etc/motd"), requirement)
    }

    /// Every ordering of `items`, by Heap's algorithm.
    fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
        fn heap<T: Clone>(k: usize, items: &mut Vec<T>, out: &mut Vec<Vec<T>>) {
            if k <= 1 {
                out.push(items.clone());
                return;
            }
            heap(k - 1, items, out);
            for i in 0..k - 1 {
                if k.is_multiple_of(2) {
                    items.swap(i, k - 1);
                } else {
                    items.swap(0, k - 1);
                }
                heap(k - 1, items, out);
            }
        }
        let mut work = items.to_vec();
        let mut out = Vec::new();
        heap(work.len(), &mut work, &mut out);
        out
    }

    #[test]
    fn evidence_truth_table() {
        let d1 = digest(1);
        let d2 = digest(2);
        let absent = FileCondition::Absent;
        let any = FileCondition::Present {
            content: Content::Any,
        };
        let exactly1 = FileCondition::Present {
            content: Content::Exactly(d1),
        };
        let table = [
            (absent, FileEvidence::Absent, Assessment::Satisfied),
            (
                absent,
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
        for (requirement, evidence, expected) in table {
            assert_eq!(
                assess_evidence(&requirement, &evidence),
                expected,
                "{requirement:?} against {evidence:?}"
            );
        }
    }

    #[test]
    fn size_does_not_affect_content_judgement() {
        let requirement = FileCondition::Present {
            content: Content::Exactly(digest(7)),
        };
        for size in [0, 1, u64::MAX] {
            let evidence = FileEvidence::Present {
                digest: digest(7),
                size,
            };
            assert_eq!(
                assess_evidence(&requirement, &evidence),
                Assessment::Satisfied
            );
        }
    }

    #[test]
    fn evidence_is_never_indeterminate() {
        for requirement in requirements() {
            for evidence in evidences() {
                assert!(!assess_evidence(&requirement, &evidence).is_indeterminate());
            }
        }
    }

    #[test]
    fn a_failed_collection_is_indeterminate() {
        // N13: unknown evidence does not imply noncompliance.
        for requirement in requirements() {
            for failure in FAILURES {
                let result = assess_collection(&requirement, &Collection::Failed(failure));
                assert_eq!(
                    result,
                    Assessment::Indeterminate(Reason::CollectionFailed(failure))
                );
                assert!(!result.is_variance());
                assert_ne!(result, Assessment::Satisfied);
            }
        }
    }

    #[test]
    fn collected_evidence_is_assessed_as_evidence() {
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
        for requirement in requirements() {
            assert_eq!(
                assess(&condition(requirement), &[]),
                Assessment::Indeterminate(Reason::NoObservation)
            );
        }
    }

    #[test]
    fn observations_of_other_resources_are_ignored() {
        let requirement = FileCondition::Absent;
        let others = [
            observe("/etc/issue", Collection::Collected(present(1))),
            observe("/etc/motd.d", Collection::Failed(CollectionFailure::Io)),
            observe("/etc", Collection::Collected(FileEvidence::Absent)),
        ];
        assert_eq!(
            assess(&condition(requirement), &others),
            Assessment::Indeterminate(Reason::NoObservation)
        );

        let mut with_own = others.to_vec();
        with_own.push(observe(
            "/etc/motd",
            Collection::Collected(FileEvidence::Absent),
        ));
        assert_eq!(
            assess(&condition(requirement), &with_own),
            Assessment::Satisfied
        );

        with_own.push(observe("/etc/motd", Collection::Collected(present(3))));
        assert_eq!(
            assess(&condition(requirement), &with_own),
            Assessment::Indeterminate(Reason::Conflicting)
        );
    }

    #[test]
    fn a_single_observation_agrees_with_assess_collection() {
        for requirement in requirements() {
            let mut collections: Vec<Collection> =
                evidences().into_iter().map(Collection::Collected).collect();
            collections.extend(FAILURES.iter().copied().map(Collection::Failed));
            for collection in collections {
                assert_eq!(
                    assess(&condition(requirement), &[observe("/etc/motd", collection)]),
                    assess_collection(&requirement, &collection)
                );
            }
        }
    }

    #[test]
    fn denied_read_is_never_satisfied_or_variance() {
        for requirement in requirements() {
            let result = assess(
                &condition(requirement),
                &[observe(
                    "/etc/motd",
                    Collection::Failed(CollectionFailure::PermissionDenied),
                )],
            );
            assert!(result.is_indeterminate());
        }
    }

    #[test]
    fn every_failure_reports_the_smallest_reason_in_any_order() {
        let observations: Vec<Observation> = FAILURES
            .iter()
            .rev()
            .map(|failure| observe("/etc/motd", Collection::Failed(*failure)))
            .collect();
        for ordering in permutations(&observations) {
            assert_eq!(
                assess(&condition(FileCondition::Absent), &ordering),
                Assessment::Indeterminate(Reason::CollectionFailed(
                    CollectionFailure::PermissionDenied
                ))
            );
        }
        // Without PermissionDenied the smallest is TimedOut.
        let rest = [
            observe("/etc/motd", Collection::Failed(CollectionFailure::Io)),
            observe("/etc/motd", Collection::Failed(CollectionFailure::TimedOut)),
            observe(
                "/etc/motd",
                Collection::Failed(CollectionFailure::Unsupported),
            ),
        ];
        for ordering in permutations(&rest) {
            assert_eq!(
                assess(&condition(FileCondition::Absent), &ordering),
                Assessment::Indeterminate(Reason::CollectionFailed(CollectionFailure::TimedOut))
            );
        }
    }

    #[test]
    fn agreeing_observations_are_sufficient() {
        // Two collectors see the same thing: the evidence is sufficient.
        let requirement = FileCondition::Present {
            content: Content::Exactly(digest(1)),
        };
        let observations = [
            Observation::file(
                path("/etc/motd"),
                Collection::Collected(present(2)),
                provenance("a", 1),
            ),
            Observation::file(
                path("/etc/motd"),
                Collection::Collected(present(2)),
                provenance("b", 5),
            ),
        ];
        assert_eq!(
            assess(&condition(requirement), &observations),
            Assessment::Variance(Variance::ContentDiffers {
                expected: digest(1),
                observed: digest(2),
            })
        );
    }

    #[test]
    fn disagreeing_observations_are_conflicting_even_when_both_would_satisfy() {
        // Interpretation: sufficiency requires that Observations of r do not
        // contradict each other, so disagreement about the evidence itself is
        // Conflicting, whatever the requirement. Content::Any would accept
        // either digest, but the evidence still contradicts itself.
        let any = FileCondition::Present {
            content: Content::Any,
        };
        let observations = [
            observe("/etc/motd", Collection::Collected(present(1))),
            observe("/etc/motd", Collection::Collected(present(2))),
        ];
        for requirement in requirements() {
            assert_eq!(
                assess(&condition(requirement), &observations),
                Assessment::Indeterminate(Reason::Conflicting)
            );
        }
        assert_eq!(
            assess(&condition(any), &observations),
            Assessment::Indeterminate(Reason::Conflicting)
        );

        let absent_and_present = [
            observe("/etc/motd", Collection::Collected(FileEvidence::Absent)),
            observe("/etc/motd", Collection::Collected(present(1))),
        ];
        for requirement in requirements() {
            assert_eq!(
                assess(&condition(requirement), &absent_and_present),
                Assessment::Indeterminate(Reason::Conflicting)
            );
        }
    }

    #[test]
    fn same_digest_different_size_is_conflicting() {
        // Regression: the two FileEvidence values differ, so they disagree.
        let observations = [
            observe(
                "/etc/motd",
                Collection::Collected(FileEvidence::Present {
                    digest: digest(1),
                    size: 1,
                }),
            ),
            observe(
                "/etc/motd",
                Collection::Collected(FileEvidence::Present {
                    digest: digest(1),
                    size: 2,
                }),
            ),
        ];
        assert_eq!(
            assess(
                &condition(FileCondition::Present {
                    content: Content::Any
                }),
                &observations
            ),
            Assessment::Indeterminate(Reason::Conflicting)
        );
    }

    #[test]
    fn a_failure_beside_collected_evidence_does_not_hide_it() {
        // Interpretation: CollectionFailed means every Observation failed. A
        // failed collection carries no evidence, so it contradicts nothing,
        // and collected evidence beside it is judged on its own.
        for requirement in requirements() {
            for evidence in evidences() {
                for failure in FAILURES {
                    let observations = [
                        observe("/etc/motd", Collection::Failed(failure)),
                        observe("/etc/motd", Collection::Collected(evidence)),
                    ];
                    for ordering in permutations(&observations) {
                        assert_eq!(
                            assess(&condition(requirement), &ordering),
                            assess_evidence(&requirement, &evidence)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn conflict_beside_a_failure_is_still_conflicting() {
        let observations = [
            observe("/etc/motd", Collection::Failed(CollectionFailure::Io)),
            observe("/etc/motd", Collection::Collected(present(1))),
            observe("/etc/motd", Collection::Collected(FileEvidence::Absent)),
        ];
        for ordering in permutations(&observations) {
            assert_eq!(
                assess(&condition(FileCondition::Absent), &ordering),
                Assessment::Indeterminate(Reason::Conflicting)
            );
        }
    }

    #[test]
    fn assess_is_order_independent_over_mixed_inputs() {
        // Metamorphic relation: permuting the Observations never changes the
        // Assessment. Checked over every ordering of small mixed sets.
        let pool = [
            Collection::Collected(FileEvidence::Absent),
            Collection::Collected(present(1)),
            Collection::Collected(present(1)),
            Collection::Failed(CollectionFailure::Unsupported),
            Collection::Failed(CollectionFailure::TimedOut),
        ];
        for mask in 1u32..(1 << pool.len()) {
            let chosen: Vec<Observation> = pool
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, collection)| observe("/etc/motd", *collection))
                .collect();
            for requirement in requirements() {
                let reference = assess(&condition(requirement), &chosen);
                for ordering in permutations(&chosen) {
                    assert_eq!(assess(&condition(requirement), &ordering), reference);
                }
            }
        }
    }

    #[test]
    fn permutations_helper_is_complete() {
        let items = [1u8, 2, 3, 4];
        let mut all = permutations(&items);
        assert_eq!(all.len(), 24);
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 24);
    }
}
