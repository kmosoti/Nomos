//! Bounded-verifier harnesses, compiled only under `cargo kani`.
//!
//! Each harness states one property of a pure predicate over every value of
//! its argument types, and each carries `kani::cover!` checks so that a
//! vacuous harness, one whose assertions hold because no path reaches them,
//! is reported as such (ADR 0007 §6). The result of a run is recorded in the
//! `assessment-algebra` result card; it establishes the property for the
//! harnessed function under Kani's model of Rust, and nothing about the
//! callers of that function.

use crate::assessment::{Assessment, Reason, assess_collection, assess_file, assess_unit};
use crate::condition::{
    Activity, Content, Enablement, FileCondition, Metadata, Requirement, UnitCondition,
};
use crate::observation::{
    Account, ActiveState, Collection, CollectionFailure, FileEvidence, ObservedMetadata,
    UnitEvidence, UnitFileState,
};
use crate::resource::Mode;

/// Any mode: every value of the twelve permission bits.
fn any_mode() -> Mode {
    match Mode::new(kani::any::<u16>() & 0o7777) {
        Some(m) => m,
        None => Mode::DEFAULT_FILE,
    }
}

/// Any file requirement whose metadata names no account: an account name
/// allocates, and the owner and group are compared by the same equality as
/// the mode, which this states over every value.
fn any_file_condition() -> FileCondition {
    if kani::any() {
        FileCondition::Absent
    } else {
        FileCondition::Present {
            content: kani::any(),
            metadata: Metadata {
                owner: None,
                group: None,
                mode: if kani::any() { Some(any_mode()) } else { None },
            },
        }
    }
}

/// Any file evidence whose owner and group are reported by number.
fn any_file_evidence() -> FileEvidence {
    if kani::any() {
        FileEvidence::Absent
    } else {
        FileEvidence::Present {
            digest: kani::any(),
            size: kani::any(),
            metadata: ObservedMetadata {
                owner: Account::Id(kani::any()),
                group: Account::Id(kani::any()),
                mode: any_mode(),
            },
        }
    }
}

/// The reference predicate: does the evidence satisfy the requirement?
/// Written as a direct reading of the file table of resource-families.md,
/// independently of the match in `assess_file`.
fn holds(requirement: &FileCondition, evidence: &FileEvidence) -> bool {
    match (requirement, evidence) {
        (FileCondition::Absent, FileEvidence::Absent) => true,
        (
            FileCondition::Present { content, metadata },
            FileEvidence::Present {
                digest,
                metadata: observed,
                ..
            },
        ) => {
            let bytes = match content {
                Content::Any => true,
                Content::Exactly(expected) => expected == digest,
            };
            bytes && metadata.mode.is_none_or(|m| m == observed.mode)
        }
        _ => false,
    }
}

/// Soundness of `assess_file`: Satisfied exactly when the evidence
/// satisfies the requirement, a Variance otherwise, and never Indeterminate.
/// The unwinding bound covers a comparison of two account names, at most
/// 32 bytes each; Kani checks that it suffices.
#[kani::proof]
#[kani::unwind(34)]
fn evidence_assessment_is_sound() {
    let requirement = any_file_condition();
    let evidence = any_file_evidence();
    let assessment = assess_file(&requirement, &evidence);
    assert_eq!(
        matches!(assessment, Assessment::Satisfied),
        holds(&requirement, &evidence)
    );
    assert!(!matches!(assessment, Assessment::Indeterminate(_)));
    kani::cover!(matches!(assessment, Assessment::Satisfied));
    kani::cover!(matches!(assessment, Assessment::Variance(_)));
}

/// The reference predicate for units: the unit table of
/// resource-families.md, one axis at a time.
fn unit_holds(requirement: &UnitCondition, evidence: &UnitEvidence) -> bool {
    let running = matches!(
        evidence.active,
        ActiveState::Active | ActiveState::Reloading
    );
    let stopped = matches!(evidence.active, ActiveState::Inactive | ActiveState::Failed);
    let activity = match requirement.activity {
        Activity::Active => running,
        Activity::Inactive => stopped,
        Activity::Any => true,
    };
    let enablement = match requirement.enablement {
        Enablement::Enabled => evidence.file_state == UnitFileState::Enabled,
        Enablement::Disabled => evidence.file_state == UnitFileState::Disabled,
        Enablement::Any => true,
    };
    activity && enablement
}

/// Soundness of `assess_unit` over every requirement and every evidence.
#[kani::proof]
fn unit_assessment_is_sound() {
    let requirement: UnitCondition = kani::any();
    let evidence: UnitEvidence = kani::any();
    let assessment = assess_unit(&requirement, &evidence);
    assert_eq!(
        matches!(assessment, Assessment::Satisfied),
        unit_holds(&requirement, &evidence)
    );
    assert!(!matches!(assessment, Assessment::Indeterminate(_)));
    kani::cover!(matches!(assessment, Assessment::Satisfied));
    kani::cover!(matches!(assessment, Assessment::Variance(_)));
}

/// N13 at the collection boundary: a failed collection is Indeterminate with
/// that failure, whatever the requirement.
#[kani::proof]
fn a_failed_collection_is_indeterminate() {
    let requirement = if kani::any() {
        Requirement::File(any_file_condition())
    } else {
        Requirement::Unit(kani::any())
    };
    let failure: CollectionFailure = kani::any();
    let assessment = assess_collection(&requirement, &Collection::Failed(failure));
    assert!(matches!(
        assessment,
        Assessment::Indeterminate(Reason::CollectionFailed(f)) if f == failure
    ));
    kani::cover!(matches!(failure, CollectionFailure::Unavailable));
    kani::cover!(matches!(requirement, Requirement::Unit(_)));
}

mod transition {
    //! Harnesses for the transition rules of milestone `05-transition-kernel`.
    //! Their result is recorded in the `kernel-conformance` result card.

    use crate::action::{Failure, Signal, Stage, Verdict, Verified, advance};
    use crate::assessment::{Reason, Variance};
    use crate::effect::{Receipt, Settlement};
    use crate::observation::Instant;
    use crate::plan::{Fence, Generation, PlanId};

    /// A `Verified` over an empty path, which allocates nothing; `verify`
    /// itself is exercised by the unit tests and the assessment harnesses.
    fn verified() -> Option<Verified> {
        Some(Verified::for_harness())
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
