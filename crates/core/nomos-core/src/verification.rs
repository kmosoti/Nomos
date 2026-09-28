//! Bounded-verifier harnesses, compiled only under `cargo kani`.
//!
//! Each harness states one property of a pure predicate over every value of
//! its argument types, and each carries `kani::cover!` checks so that a
//! vacuous harness, one whose assertions hold because no path reaches them,
//! is reported as such (ADR 0007 §6). The result of a run is recorded in the
//! `assessment-algebra` result card; it establishes the property for the
//! harnessed function under Kani's model of Rust, and nothing about the
//! callers of that function.

use crate::assessment::{Assessment, Reason, assess_collection, assess_evidence};
use crate::condition::{Content, FileCondition};
use crate::observation::{Collection, CollectionFailure, FileEvidence};

/// The reference predicate: does the evidence satisfy the requirement?
/// Written as a direct reading of spec §3, independently of the match in
/// `assess_evidence`.
fn holds(requirement: &FileCondition, evidence: &FileEvidence) -> bool {
    match requirement {
        FileCondition::Absent => matches!(evidence, FileEvidence::Absent),
        FileCondition::Present {
            content: Content::Any,
        } => matches!(evidence, FileEvidence::Present { .. }),
        FileCondition::Present {
            content: Content::Exactly(expected),
        } => matches!(evidence, FileEvidence::Present { digest, .. } if digest == expected),
    }
}

/// Soundness of `assess_evidence`: Satisfied exactly when the evidence
/// satisfies the requirement, a Variance otherwise, and never Indeterminate.
#[kani::proof]
fn evidence_assessment_is_sound() {
    let requirement: FileCondition = kani::any();
    let evidence: FileEvidence = kani::any();
    let assessment = assess_evidence(&requirement, &evidence);
    assert_eq!(
        matches!(assessment, Assessment::Satisfied),
        holds(&requirement, &evidence)
    );
    assert!(!matches!(assessment, Assessment::Indeterminate(_)));
    kani::cover!(matches!(assessment, Assessment::Satisfied));
    kani::cover!(matches!(assessment, Assessment::Variance(_)));
}

/// N13 at the collection boundary: a failed collection is Indeterminate with
/// that failure, whatever the requirement.
#[kani::proof]
fn a_failed_collection_is_indeterminate() {
    let requirement: FileCondition = kani::any();
    let failure: CollectionFailure = kani::any();
    let assessment = assess_collection(&requirement, &Collection::Failed(failure));
    assert!(matches!(
        assessment,
        Assessment::Indeterminate(Reason::CollectionFailed(f)) if f == failure
    ));
    kani::cover!(matches!(failure, CollectionFailure::PermissionDenied));
    kani::cover!(matches!(requirement, FileCondition::Present { .. }));
}
