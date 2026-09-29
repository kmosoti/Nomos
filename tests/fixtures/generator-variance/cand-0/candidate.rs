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
        (FileCondition::Present { content }, FileEvidence::Present { digest, .. }) => match content
        {
            Content::Any => Assessment::Satisfied,
            Content::Exactly(expected) if expected == digest => Assessment::Satisfied,
            Content::Exactly(expected) => Assessment::Variance(Variance::ContentDiffers {
                expected: *expected,
                observed: *digest,
            }),
        },
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
///
/// NEGATIVE CONTROL. This function is wrong on purpose, in the way
/// `SM-ASSESS-001` names: when every Observation of the resource failed, it
/// returns `Variance(Missing)` instead of `Indeterminate(CollectionFailed)`.
pub fn assess(condition: &Condition, observations: &[Observation]) -> Assessment {
    let mut collected: Vec<FileEvidence> = Vec::new();
    let mut failed = false;
    for observation in observations.iter().filter(|o| o.path() == condition.path()) {
        match observation.collection() {
            Collection::Collected(evidence) => collected.push(*evidence),
            Collection::Failed(_) => failed = true,
        }
    }
    match collected.split_first() {
        Some((first, rest)) if rest.iter().all(|e| e == first) => {
            assess_evidence(condition.requirement(), first)
        }
        Some(_) => Assessment::Indeterminate(Reason::Conflicting),
        // The deliberate defect: no evidence was collected, and the
        // resource is treated as missing.
        None if failed => Assessment::Variance(Variance::Missing),
        None => Assessment::Indeterminate(Reason::NoObservation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::observation::{CollectorId, Instant, Provenance, Window};
    use crate::resource::ResourcePath;

    fn observed(at: &str, collection: Collection) -> Observation {
        Observation::file(
            ResourcePath::new(at).unwrap(),
            collection,
            Provenance::new(
                CollectorId::new("control").unwrap(),
                Window::new(Instant(0), Instant(1)).unwrap(),
            ),
        )
    }

    fn condition(at: &str, requirement: FileCondition) -> Condition {
        Condition::file(ResourcePath::new(at).unwrap(), requirement)
    }

    // Regression test of the deliberate defect: it agrees with the code.
    #[test]
    fn a_failed_observation_is_a_missing_file() {
        let c = condition(
            "/etc/hosts",
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let o = observed(
            "/etc/hosts",
            Collection::Failed(CollectionFailure::PermissionDenied),
        );
        assert_eq!(assess(&c, &[o]), Assessment::Variance(Variance::Missing));
    }

    #[test]
    fn a_present_file_satisfies_any_content() {
        let c = condition(
            "/etc/hosts",
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let o = observed(
            "/etc/hosts",
            Collection::Collected(FileEvidence::Present {
                digest: Digest::from_bytes([1; 32]),
                size: 1,
            }),
        );
        assert_eq!(assess(&c, &[o]), Assessment::Satisfied);
    }

    #[test]
    fn nothing_observed_is_indeterminate() {
        let c = condition("/etc/hosts", FileCondition::Absent);
        assert_eq!(
            assess(&c, &[]),
            Assessment::Indeterminate(Reason::NoObservation)
        );
    }
}
