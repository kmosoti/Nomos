//! The Action lifecycle (spec §18) and verification (N6, N10).
//!
//! ```text
//! Prepared → Dispatched → Accepted → Running → Verifying → Succeeded
//! ```
//!
//! with the terminal alternatives Failed, TimedOut, Cancelled, and Rejected.
//! [`advance`] is the whole transition relation; a pair it does not list is
//! an error, never a guess.
//!
//! - **N6.** `Succeeded` holds a [`Verified`], and the only constructor of a
//!   `Verified` is [`verify`], which returns one only when a fresh
//!   Observation assesses the Condition as Satisfied. The only transition
//!   into `Succeeded` is from `Verifying`.
//! - **N10.** A deadline in a live stage gives `TimedOut`, never `Failed`,
//!   and nothing leaves `TimedOut`: a timed-out outcome is neither success
//!   nor failure, and the next decision about the resource comes from a new
//!   Observation. The effect's reservation is a separate matter, settled by
//!   [`crate::effect::Settlement`], so a timeout frees nothing.

use alloc::vec::Vec;
use core::fmt;

use crate::assessment::{Assessment, Reason, Variance, assess};
use crate::condition::Condition;
use crate::effect::{Receipt, Settlement};
use crate::observation::{Instant, Observation};
use crate::resource::ResourcePath;

/// Evidence that a postcondition held on a fresh Observation. Only
/// [`verify`] builds one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Verified {
    resource: ResourcePath,
    since: Instant,
}

impl Verified {
    /// The resource whose Condition held.
    pub fn resource(&self) -> &ResourcePath {
        &self.resource
    }

    /// The instant the evidence was fresh from.
    pub fn since(&self) -> Instant {
        self.since
    }
}

/// The outcome of verifying a postcondition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verdict {
    /// The Condition holds on fresh evidence.
    Holds(Verified),
    /// Fresh evidence shows this Variance.
    Fails(Variance),
    /// No fresh, sufficient evidence. Nothing is established.
    Unknown(Reason),
}

/// Verifies `condition` against the Observations collected at or after
/// `since`, the instant the effect was reported complete. An Observation
/// whose collection window starts earlier says nothing about the effect and
/// is ignored (reconciliation.md, Verification).
pub fn verify(condition: &Condition, observations: &[Observation], since: Instant) -> Verdict {
    let fresh: Vec<Observation> = observations
        .iter()
        .filter(|o| o.path() == condition.path() && o.provenance().window().start() >= since)
        .cloned()
        .collect();
    match assess(condition, &fresh) {
        Assessment::Satisfied => Verdict::Holds(Verified {
            resource: condition.path().clone(),
            since,
        }),
        Assessment::Variance(variance) => Verdict::Fails(variance),
        Assessment::Indeterminate(reason) => Verdict::Unknown(reason),
    }
}

/// Why an Action failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Failure {
    /// The Substrate reported the effect failed.
    Effect,
    /// The effect completed, and fresh evidence shows the Condition still
    /// does not hold.
    Postcondition(Variance),
}

/// Where an Action is in its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    /// Planned, not yet dispatched.
    Prepared,
    /// The effect request was issued.
    Dispatched,
    /// The Substrate accepted the request.
    Accepted,
    /// The effect started.
    Running,
    /// The effect completed; its postcondition is being verified.
    Verifying {
        /// Whether the effect reported changing the resource.
        changed: bool,
    },
    /// The postcondition was verified.
    Succeeded {
        /// Whether the effect changed the resource.
        changed: bool,
        /// The evidence.
        verified: Verified,
    },
    /// The outcome is known and is a failure.
    Failed(Failure),
    /// A deadline passed with the outcome unknown.
    TimedOut,
    /// Withdrawn before dispatch.
    Cancelled,
    /// The Substrate refused the request.
    Rejected,
}

impl Stage {
    /// Dispatched and not yet terminal: the Action holds its reservation
    /// whatever its effect's settlement.
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            Stage::Dispatched | Stage::Accepted | Stage::Running | Stage::Verifying { .. }
        )
    }

    /// No transition leaves this stage.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Stage::Succeeded { .. }
                | Stage::Failed(_)
                | Stage::TimedOut
                | Stage::Cancelled
                | Stage::Rejected
        )
    }

    /// Terminal with a known outcome. `TimedOut` is terminal and unknown.
    pub fn outcome_known(&self) -> bool {
        self.is_terminal() && !matches!(self, Stage::TimedOut)
    }
}

/// Whether an Action's conflict keys stay reserved: while its effect is not
/// Settled, or while the Action is live, whichever lasts longer
/// ([warp.md](../../../../docs/formal/warp.md#conflict-keys), ADR 0009 §5).
/// A timeout ends the Action, not the effect, so a TimedOut Action with an
/// unsettled effect still holds its keys. `stage` is `None` for an effect
/// whose Action belongs to a run that is over.
pub fn holds_reservation(stage: Option<&Stage>, settlement: &Settlement) -> bool {
    !settlement.is_settled() || stage.is_some_and(Stage::is_live)
}

/// Something that happens to an Action.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Signal {
    /// The kernel issues the effect request.
    Dispatch,
    /// The Substrate accepts the request.
    Accept,
    /// The effect starts.
    Start,
    /// The effect completes.
    Complete {
        /// Whether it changed the resource.
        changed: bool,
    },
    /// A verification verdict arrives.
    Verify(Verdict),
    /// The effect fails.
    Fail,
    /// The Substrate refuses the request.
    Refuse,
    /// The Action is withdrawn before dispatch.
    Cancel,
    /// The receipt or verification deadline passes.
    Deadline,
}

/// Why a signal could not be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// The stage is terminal; nothing leaves it.
    Terminal,
    /// The signal does not apply to the stage.
    NotEnabled,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TransitionError::Terminal => "the Action is terminal",
            TransitionError::NotEnabled => "the signal does not apply in this stage",
        })
    }
}

/// The lifecycle transition relation.
pub fn advance(stage: &Stage, signal: Signal) -> Result<Stage, TransitionError> {
    if stage.is_terminal() {
        return Err(TransitionError::Terminal);
    }
    match (stage, signal) {
        (Stage::Prepared, Signal::Dispatch) => Ok(Stage::Dispatched),
        (Stage::Prepared, Signal::Cancel) => Ok(Stage::Cancelled),
        (Stage::Dispatched, Signal::Accept) => Ok(Stage::Accepted),
        (Stage::Accepted, Signal::Start) => Ok(Stage::Running),
        (Stage::Running, Signal::Complete { changed }) => Ok(Stage::Verifying { changed }),
        (Stage::Verifying { changed }, Signal::Verify(verdict)) => Ok(match verdict {
            Verdict::Holds(verified) => Stage::Succeeded {
                changed: *changed,
                verified,
            },
            Verdict::Fails(variance) => Stage::Failed(Failure::Postcondition(variance)),
            Verdict::Unknown(_) => Stage::Verifying { changed: *changed },
        }),
        (Stage::Accepted | Stage::Running, Signal::Fail) => Ok(Stage::Failed(Failure::Effect)),
        (Stage::Dispatched | Stage::Accepted, Signal::Refuse) => Ok(Stage::Rejected),
        (live, Signal::Deadline) if live.is_live() => Ok(Stage::TimedOut),
        _ => Err(TransitionError::NotEnabled),
    }
}

/// The signals a receipt stands for from `stage`. Receipts can arrive out of
/// order or be lost, and a completion implies acceptance and start, so a
/// completion that finds the Action `Dispatched` stands for all three. A
/// receipt that implies nothing from `stage`, such as a late start after
/// completion, stands for none.
pub fn signals_for(stage: &Stage, receipt: &Receipt) -> Vec<Signal> {
    use Signal::{Accept, Complete, Fail, Refuse, Start};
    match (stage, *receipt) {
        (Stage::Dispatched, Receipt::Accepted) => alloc::vec![Accept],
        (Stage::Dispatched, Receipt::Started) => alloc::vec![Accept, Start],
        (Stage::Accepted, Receipt::Started) => alloc::vec![Start],
        (Stage::Dispatched, Receipt::Completed { changed }) => {
            alloc::vec![Accept, Start, Complete { changed }]
        }
        (Stage::Accepted, Receipt::Completed { changed }) => {
            alloc::vec![Start, Complete { changed }]
        }
        (Stage::Running, Receipt::Completed { changed }) => alloc::vec![Complete { changed }],
        (Stage::Dispatched, Receipt::Failed) => alloc::vec![Accept, Fail],
        (Stage::Accepted | Stage::Running, Receipt::Failed) => alloc::vec![Fail],
        (Stage::Dispatched | Stage::Accepted, Receipt::Refused) => alloc::vec![Refuse],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::condition::{Content, FileCondition};
    use crate::observation::{Collection, CollectorId, FileEvidence, Provenance, Window};
    use crate::resource::Digest;
    use alloc::vec;

    fn path() -> ResourcePath {
        ResourcePath::new("/etc/app.conf").unwrap()
    }

    fn condition() -> Condition {
        Condition::file(
            path(),
            FileCondition::Present {
                content: Content::Exactly(Digest::from_bytes([7; 32])),
            },
        )
    }

    fn observed(evidence: Collection, start: u64) -> Observation {
        Observation::file(
            path(),
            evidence,
            Provenance::new(
                CollectorId::new("test").unwrap(),
                Window::new(Instant(start), Instant(start + 1)).unwrap(),
            ),
        )
    }

    fn matching(start: u64) -> Observation {
        observed(
            Collection::Collected(FileEvidence::Present {
                digest: Digest::from_bytes([7; 32]),
                size: 1,
            }),
            start,
        )
    }

    fn verified() -> Verified {
        match verify(&condition(), &[matching(5)], Instant(5)) {
            Verdict::Holds(v) => v,
            other => panic!("expected Holds, got {other:?}"),
        }
    }

    /// Every stage, one representative per variant.
    fn stages() -> Vec<Stage> {
        vec![
            Stage::Prepared,
            Stage::Dispatched,
            Stage::Accepted,
            Stage::Running,
            Stage::Verifying { changed: true },
            Stage::Verifying { changed: false },
            Stage::Succeeded {
                changed: true,
                verified: verified(),
            },
            Stage::Failed(Failure::Effect),
            Stage::Failed(Failure::Postcondition(Variance::Missing)),
            Stage::TimedOut,
            Stage::Cancelled,
            Stage::Rejected,
        ]
    }

    /// Every signal, one representative per variant and verdict.
    fn signals() -> Vec<Signal> {
        vec![
            Signal::Dispatch,
            Signal::Accept,
            Signal::Start,
            Signal::Complete { changed: true },
            Signal::Complete { changed: false },
            Signal::Verify(Verdict::Holds(verified())),
            Signal::Verify(Verdict::Fails(Variance::Missing)),
            Signal::Verify(Verdict::Unknown(Reason::NoObservation)),
            Signal::Fail,
            Signal::Refuse,
            Signal::Cancel,
            Signal::Deadline,
        ]
    }

    /// The transition table of spec §18, written from the specification and
    /// not from `advance`: every enabled pair and its target. Any pair not
    /// listed must be refused.
    fn expected(stage: &Stage, signal: &Signal) -> Option<Stage> {
        use Stage as S;
        let live = matches!(
            stage,
            S::Dispatched | S::Accepted | S::Running | S::Verifying { .. }
        );
        match (stage, signal) {
            (S::Prepared, Signal::Dispatch) => Some(S::Dispatched),
            (S::Prepared, Signal::Cancel) => Some(S::Cancelled),
            (S::Dispatched, Signal::Accept) => Some(S::Accepted),
            (S::Accepted, Signal::Start) => Some(S::Running),
            (S::Running, Signal::Complete { changed }) => Some(S::Verifying { changed: *changed }),
            (S::Verifying { changed }, Signal::Verify(Verdict::Holds(v))) => Some(S::Succeeded {
                changed: *changed,
                verified: v.clone(),
            }),
            (S::Verifying { .. }, Signal::Verify(Verdict::Fails(variance))) => {
                Some(S::Failed(Failure::Postcondition(*variance)))
            }
            (S::Verifying { changed }, Signal::Verify(Verdict::Unknown(_))) => {
                Some(S::Verifying { changed: *changed })
            }
            (S::Accepted, Signal::Fail) | (S::Running, Signal::Fail) => {
                Some(S::Failed(Failure::Effect))
            }
            (S::Dispatched, Signal::Refuse) | (S::Accepted, Signal::Refuse) => Some(S::Rejected),
            (_, Signal::Deadline) if live => Some(S::TimedOut),
            _ => None,
        }
    }

    #[test]
    fn the_lifecycle_table_is_exhaustive() {
        let mut enabled = 0;
        for stage in stages() {
            for signal in signals() {
                let got = advance(&stage, signal.clone());
                match expected(&stage, &signal) {
                    Some(target) => {
                        enabled += 1;
                        assert_eq!(got, Ok(target), "{stage:?} + {signal:?}");
                    }
                    None if stage.is_terminal() => {
                        assert_eq!(
                            got,
                            Err(TransitionError::Terminal),
                            "{stage:?} + {signal:?}"
                        )
                    }
                    None => {
                        assert_eq!(
                            got,
                            Err(TransitionError::NotEnabled),
                            "{stage:?} + {signal:?}"
                        )
                    }
                }
            }
        }
        assert_eq!(enabled, 21);
    }

    /// N6, and `SM-TRANSITION-001`'s named test: the effect's completion
    /// leads to Verifying, and the only way into Succeeded is from Verifying
    /// with a verdict that holds.
    #[test]
    fn succeeded_requires_verifying() {
        assert_eq!(
            advance(&Stage::Running, Signal::Complete { changed: true }),
            Ok(Stage::Verifying { changed: true })
        );
        for stage in stages() {
            for signal in signals() {
                if let Ok(Stage::Succeeded { .. }) = advance(&stage, signal.clone()) {
                    assert!(
                        matches!(stage, Stage::Verifying { .. })
                            && matches!(signal, Signal::Verify(Verdict::Holds(_))),
                        "{stage:?} + {signal:?} reached Succeeded"
                    );
                }
            }
        }
    }

    /// N10, and `SM-TRANSITION-002`'s named test: a deadline in any live
    /// stage gives TimedOut, which is neither Succeeded nor Failed, has an
    /// unknown outcome, and never leaves.
    #[test]
    fn timed_out_stays_unknown() {
        for stage in stages().into_iter().filter(Stage::is_live) {
            assert_eq!(
                advance(&stage, Signal::Deadline),
                Ok(Stage::TimedOut),
                "{stage:?}"
            );
        }
        assert!(!Stage::TimedOut.outcome_known());
        assert!(Stage::TimedOut.is_terminal());
        assert!(!Stage::TimedOut.is_live());
        for signal in signals() {
            assert_eq!(
                advance(&Stage::TimedOut, signal.clone()),
                Err(TransitionError::Terminal),
                "{signal:?}"
            );
        }
    }

    /// The reservation rule, written out: held while unsettled or live.
    #[test]
    fn a_reservation_outlives_a_timeout() {
        use crate::effect::Settlement;
        let unsettled = Settlement::new(Instant(10));
        let settled = unsettled.on_receipt(&Receipt::Failed);
        for stage in stages() {
            assert!(holds_reservation(Some(&stage), &unsettled), "{stage:?}");
            assert_eq!(
                holds_reservation(Some(&stage), &settled),
                stage.is_live(),
                "{stage:?}"
            );
        }
        assert!(holds_reservation(Some(&Stage::TimedOut), &unsettled));
        assert!(holds_reservation(None, &unsettled));
        assert!(!holds_reservation(None, &settled));
    }

    #[test]
    fn stage_classes_partition_the_stages() {
        for stage in stages() {
            let live = stage.is_live();
            let terminal = stage.is_terminal();
            assert!(!(live && terminal), "{stage:?}");
            assert_eq!(live || terminal, stage != Stage::Prepared, "{stage:?}");
            assert_eq!(
                stage.outcome_known(),
                terminal && stage != Stage::TimedOut,
                "{stage:?}"
            );
        }
    }

    #[test]
    fn verification_ignores_evidence_older_than_the_completion() {
        let stale = matching(4);
        assert_eq!(
            verify(&condition(), core::slice::from_ref(&stale), Instant(5)),
            Verdict::Unknown(Reason::NoObservation)
        );
        assert!(matches!(
            verify(&condition(), &[stale, matching(5)], Instant(5)),
            Verdict::Holds(_)
        ));
    }

    #[test]
    fn a_fresh_variance_fails_and_a_failed_read_establishes_nothing() {
        let absent = observed(Collection::Collected(FileEvidence::Absent), 6);
        assert_eq!(
            verify(&condition(), &[absent], Instant(5)),
            Verdict::Fails(Variance::Missing)
        );
        let denied = observed(
            Collection::Failed(crate::observation::CollectionFailure::PermissionDenied),
            6,
        );
        assert!(matches!(
            verify(&condition(), &[denied], Instant(5)),
            Verdict::Unknown(Reason::CollectionFailed(_))
        ));
    }

    #[test]
    fn verified_names_its_resource_and_instant() {
        let v = verified();
        assert_eq!(v.resource(), &path());
        assert_eq!(v.since(), Instant(5));
    }

    /// Receipts can arrive out of order; each stands for the signals that
    /// carry the Action to it, and applying them is always legal.
    #[test]
    fn a_receipt_stands_for_the_signals_that_reach_it() {
        use Signal as G;
        let done = |changed| G::Complete { changed };
        let cases: Vec<(Stage, Receipt, Vec<Signal>)> = vec![
            (Stage::Dispatched, Receipt::Accepted, vec![G::Accept]),
            (
                Stage::Dispatched,
                Receipt::Started,
                vec![G::Accept, G::Start],
            ),
            (
                Stage::Dispatched,
                Receipt::Completed { changed: true },
                vec![G::Accept, G::Start, done(true)],
            ),
            (Stage::Dispatched, Receipt::Failed, vec![G::Accept, G::Fail]),
            (Stage::Dispatched, Receipt::Refused, vec![G::Refuse]),
            (Stage::Accepted, Receipt::Accepted, vec![]),
            (Stage::Accepted, Receipt::Started, vec![G::Start]),
            (
                Stage::Accepted,
                Receipt::Completed { changed: false },
                vec![G::Start, done(false)],
            ),
            (Stage::Accepted, Receipt::Failed, vec![G::Fail]),
            (Stage::Accepted, Receipt::Refused, vec![G::Refuse]),
            (Stage::Running, Receipt::Accepted, vec![]),
            (Stage::Running, Receipt::Started, vec![]),
            (
                Stage::Running,
                Receipt::Completed { changed: true },
                vec![done(true)],
            ),
            (Stage::Running, Receipt::Failed, vec![G::Fail]),
            (Stage::Running, Receipt::Refused, vec![]),
        ];
        for (stage, receipt, want) in cases {
            let got = signals_for(&stage, &receipt);
            assert_eq!(got, want, "{stage:?} + {receipt:?}");
            let mut at = stage.clone();
            for signal in got {
                at = advance(&at, signal).unwrap();
            }
        }
        for stage in [
            Stage::Prepared,
            Stage::Verifying { changed: true },
            Stage::TimedOut,
            Stage::Rejected,
        ] {
            for receipt in [
                Receipt::Accepted,
                Receipt::Started,
                Receipt::Completed { changed: true },
                Receipt::Failed,
                Receipt::Refused,
            ] {
                assert!(
                    signals_for(&stage, &receipt).is_empty(),
                    "{stage:?} + {receipt:?}"
                );
            }
        }
    }

    #[test]
    fn errors_say_what_they_are() {
        assert_eq!(
            alloc::format!("{}", TransitionError::Terminal),
            "the Action is terminal"
        );
        assert_eq!(
            alloc::format!("{}", TransitionError::NotEnabled),
            "the signal does not apply in this stage"
        );
    }
}
