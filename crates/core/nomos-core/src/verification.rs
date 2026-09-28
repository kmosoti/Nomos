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

mod transition {
    //! Harnesses for the transition rules of milestone `05-transition-kernel`.
    //! Their result is recorded in the `kernel-conformance` result card.

    use crate::action::{Failure, Signal, Stage, Verdict, Verified, advance, verify};
    use crate::assessment::{Reason, Variance};
    use crate::condition::{Condition, Content, FileCondition};
    use crate::effect::{Receipt, Settlement};
    use crate::observation::Instant;
    use crate::observation::{
        Collection, CollectorId, FileEvidence, Observation, Provenance, Window,
    };
    use crate::plan::{Fence, Generation, PlanId};
    use crate::resource::ResourcePath;

    /// A `Verified`, built the only way there is: through `verify`.
    fn verified() -> Option<Verified> {
        let resource = ResourcePath::new("/a").ok()?;
        let condition = Condition::file(
            resource.clone(),
            FileCondition::Present {
                content: Content::Any,
            },
        );
        let window = Window::new(Instant(0), Instant(0)).ok()?;
        let present = Observation::file(
            resource,
            Collection::Collected(FileEvidence::Present {
                digest: crate::resource::Digest::from_bytes([0; 32]),
                size: 0,
            }),
            Provenance::new(CollectorId::new("k")?, window),
        );
        match verify(&condition, &[present], Instant(0)) {
            Verdict::Holds(v) => Some(v),
            _ => None,
        }
    }

    fn any_stage(verified: &Verified) -> Stage {
        let changed: bool = kani::any();
        match kani::any::<u8>() % 10 {
            0 => Stage::Prepared,
            1 => Stage::Dispatched,
            2 => Stage::Accepted,
            3 => Stage::Running,
            4 => Stage::Verifying { changed },
            5 => Stage::Succeeded {
                changed,
                verified: verified.clone(),
            },
            6 => Stage::Failed(if kani::any() {
                Failure::Effect
            } else {
                Failure::Postcondition(Variance::Missing)
            }),
            7 => Stage::TimedOut,
            8 => Stage::Cancelled,
            _ => Stage::Rejected,
        }
    }

    fn any_signal(verified: &Verified) -> Signal {
        let changed: bool = kani::any();
        match kani::any::<u8>() % 11 {
            0 => Signal::Dispatch,
            1 => Signal::Accept,
            2 => Signal::Start,
            3 => Signal::Complete { changed },
            4 => Signal::Verify(Verdict::Holds(verified.clone())),
            5 => Signal::Verify(Verdict::Fails(Variance::Unexpected)),
            6 => Signal::Verify(Verdict::Unknown(Reason::NoObservation)),
            7 => Signal::Fail,
            8 => Signal::Refuse,
            9 => Signal::Cancel,
            _ => Signal::Deadline,
        }
    }

    /// N6: every path into Succeeded comes from Verifying with a verdict
    /// that holds, and carries that verdict's evidence.
    #[kani::proof]
    fn succeeded_only_from_a_holding_verification() {
        let Some(v) = verified() else { return };
        let stage = any_stage(&v);
        let signal = any_signal(&v);
        let from_verifying = matches!(stage, Stage::Verifying { .. });
        let holds = matches!(signal, Signal::Verify(Verdict::Holds(_)));
        if let Ok(Stage::Succeeded { .. }) = advance(&stage, signal) {
            assert!(from_verifying && holds);
        }
        kani::cover!(from_verifying && holds);
    }

    /// N10: a deadline never yields a known outcome, and TimedOut and every
    /// other terminal stage are absorbing.
    #[kani::proof]
    fn deadlines_establish_nothing_and_terminal_stages_absorb() {
        let Some(v) = verified() else { return };
        let stage = any_stage(&v);
        let signal = any_signal(&v);
        let deadline = matches!(signal, Signal::Deadline);
        let terminal = stage.is_terminal();
        let live = stage.is_live();
        match advance(&stage, signal) {
            Ok(next) => {
                assert!(!terminal);
                if deadline {
                    assert!(live && next == Stage::TimedOut);
                }
            }
            Err(_) => assert!(!(deadline && live)),
        }
        kani::cover!(deadline && live);
        kani::cover!(terminal);
    }

    /// N5: the accepted generation never decreases, and a stale Plan is never
    /// permitted.
    #[kani::proof]
    fn the_fence_never_moves_backwards() {
        let (Some(a), Some(b)) = (PlanId::new("a"), PlanId::new("b")) else {
            return;
        };
        let accepted = Generation(kani::any::<u8>() as u64);
        let current = if kani::any() { a.clone() } else { b.clone() };
        let Ok((fence, _)) = Fence::new().accept(accepted, &current) else {
            return;
        };
        let offered = Generation(kani::any::<u8>() as u64);
        let plan = if kani::any() { a } else { b };
        match fence.accept(offered, &plan) {
            Ok((after, _)) => {
                assert!(after.accepted().is_some_and(|(g, _)| g >= accepted));
                assert!(offered >= accepted);
            }
            Err(_) => assert!(offered <= accepted),
        }
        assert!(!fence.permits(offered, &plan) || offered == accepted);
        kani::cover!(offered < accepted);
        kani::cover!(offered > accepted);
    }

    /// Settlement: only a terminal receipt or the settle-by instant settles,
    /// and Settled stays Settled.
    #[kani::proof]
    fn settlement_needs_evidence_and_is_permanent() {
        let settle_by = Instant(kani::any::<u8>() as u64);
        let now = Instant(kani::any::<u8>() as u64);
        let receipt = match kani::any::<u8>() % 5 {
            0 => Receipt::Accepted,
            1 => Receipt::Started,
            2 => Receipt::Completed {
                changed: kani::any(),
            },
            3 => Receipt::Failed,
            _ => Receipt::Refused,
        };
        let start = Settlement::new(settle_by);
        let after_receipt = start.on_receipt(&receipt);
        assert_eq!(after_receipt.is_settled(), receipt.settles());
        let after_tick = start.at(now);
        assert_eq!(after_tick.is_settled(), now >= settle_by);
        assert!(after_tick.on_receipt(&receipt).is_settled() || !after_tick.is_settled());
        if after_receipt.is_settled() {
            assert!(after_receipt.at(now).is_settled());
        }
        kani::cover!(!receipt.settles() && now < settle_by);
    }
}
