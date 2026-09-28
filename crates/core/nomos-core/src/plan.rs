//! Plan identity and the fence that keeps a stale Plan from executing (N5).
//!
//! A Cell persists the highest generation it has accepted and the Plan it
//! accepted at that generation ([fencing-and-idempotency.md]). A Plan below
//! that generation is stale; a different Plan at that generation conflicts;
//! the same Plan again is a redelivery. Effect admission asks
//! [`Fence::permits`] in the same step that admits the effect, so nothing can
//! be accepted between the check and the admission (ADR 0010 §3).
//!
//! [fencing-and-idempotency.md]: ../../../../docs/formal/fencing-and-idempotency.md

use alloc::string::String;
use core::fmt;

/// A generation assigned by an authority. Higher supersedes lower.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(pub u64);

/// The identifier of a Plan, unique per authority.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanId(String);

impl PlanId {
    /// A Plan identifier, or `None` when `text` is empty.
    pub fn new(text: &str) -> Option<Self> {
        (!text.is_empty()).then(|| PlanId(String::from(text)))
    }

    /// The identifier as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PlanId({})", self.0)
    }
}

impl fmt::Display for PlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a Plan was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceError {
    /// The Plan's generation is below the accepted one.
    Stale {
        /// The generation the Plan carried.
        offered: Generation,
        /// The generation already accepted.
        accepted: Generation,
    },
    /// Another Plan was accepted at this generation.
    Conflict {
        /// The contested generation.
        generation: Generation,
    },
}

impl fmt::Display for FenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FenceError::Stale { offered, accepted } => write!(
                f,
                "stale Plan: generation {} is below the accepted {}",
                offered.0, accepted.0
            ),
            FenceError::Conflict { generation } => write!(
                f,
                "conflicting Plan: another Plan holds generation {}",
                generation.0
            ),
        }
    }
}

/// How an accepted Plan relates to what was accepted before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acceptance {
    /// The Plan supersedes, or is the first.
    New,
    /// The same Plan at the same generation, delivered again.
    Redelivered,
}

/// The highest accepted generation and the Plan accepted at it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fence {
    accepted: Option<(Generation, PlanId)>,
}

impl Fence {
    /// A fence that has accepted nothing.
    pub fn new() -> Self {
        Fence { accepted: None }
    }

    /// The accepted generation and Plan, if any.
    pub fn accepted(&self) -> Option<(Generation, &PlanId)> {
        self.accepted.as_ref().map(|(g, id)| (*g, id))
    }

    /// Decides whether the Plan `plan` at `generation` is accepted, and the
    /// fence that results. The fence never moves backwards.
    pub fn accept(
        &self,
        generation: Generation,
        plan: &PlanId,
    ) -> Result<(Fence, Acceptance), FenceError> {
        match &self.accepted {
            None => Ok((Fence::at(generation, plan), Acceptance::New)),
            Some((accepted, _)) if generation < *accepted => Err(FenceError::Stale {
                offered: generation,
                accepted: *accepted,
            }),
            Some((accepted, current)) if generation == *accepted => {
                if current == plan {
                    Ok((self.clone(), Acceptance::Redelivered))
                } else {
                    Err(FenceError::Conflict { generation })
                }
            }
            Some(_) => Ok((Fence::at(generation, plan), Acceptance::New)),
        }
    }

    /// Whether an effect of `plan` at `generation` may be admitted now: only
    /// the accepted Plan at the accepted generation.
    pub fn permits(&self, generation: Generation, plan: &PlanId) -> bool {
        matches!(&self.accepted, Some((g, id)) if *g == generation && id == plan)
    }

    fn at(generation: Generation, plan: &PlanId) -> Fence {
        Fence {
            accepted: Some((generation, plan.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> PlanId {
        PlanId::new(text).unwrap()
    }

    fn fence(generation: u64, plan: &str) -> Fence {
        Fence::new()
            .accept(Generation(generation), &id(plan))
            .unwrap()
            .0
    }

    /// N5, and `SM-TRANSITION-003`'s named test. The oracle is the fencing
    /// algorithm in fencing-and-idempotency.md: `P.g < g_acc` is rejected.
    #[test]
    fn a_stale_generation_is_rejected() {
        let fence = fence(52, "b");
        assert_eq!(
            fence.accept(Generation(51), &id("a")),
            Err(FenceError::Stale {
                offered: Generation(51),
                accepted: Generation(52)
            })
        );
        assert_eq!(
            fence.accept(Generation(51), &id("b")),
            Err(FenceError::Stale {
                offered: Generation(51),
                accepted: Generation(52)
            })
        );
        assert!(matches!(
            fence.accept(Generation(0), &id("a")),
            Err(FenceError::Stale { .. })
        ));
        assert!(!fence.permits(Generation(51), &id("b")));
    }

    #[test]
    fn a_different_plan_at_the_accepted_generation_conflicts() {
        let fence = fence(7, "a");
        assert_eq!(
            fence.accept(Generation(7), &id("b")),
            Err(FenceError::Conflict {
                generation: Generation(7)
            })
        );
    }

    #[test]
    fn the_same_plan_again_is_a_redelivery() {
        let fence = fence(7, "a");
        let (after, acceptance) = fence.accept(Generation(7), &id("a")).unwrap();
        assert_eq!(acceptance, Acceptance::Redelivered);
        assert_eq!(after, fence);
    }

    #[test]
    fn a_newer_generation_supersedes() {
        let fence = fence(7, "a");
        let (after, acceptance) = fence.accept(Generation(8), &id("b")).unwrap();
        assert_eq!(acceptance, Acceptance::New);
        assert_eq!(after.accepted(), Some((Generation(8), &id("b"))));
        assert!(!after.permits(Generation(7), &id("a")));
        assert!(after.permits(Generation(8), &id("b")));
    }

    #[test]
    fn an_empty_fence_permits_nothing() {
        assert!(!Fence::new().permits(Generation(0), &id("a")));
        assert!(PlanId::new("").is_none());
    }

    #[test]
    fn errors_say_what_they_are() {
        let stale = FenceError::Stale {
            offered: Generation(1),
            accepted: Generation(2),
        };
        assert_eq!(
            alloc::format!("{stale}"),
            "stale Plan: generation 1 is below the accepted 2"
        );
        let conflict = FenceError::Conflict {
            generation: Generation(3),
        };
        assert_eq!(
            alloc::format!("{conflict}"),
            "conflicting Plan: another Plan holds generation 3"
        );
        assert_eq!(alloc::format!("{:?}", id("p")), "PlanId(p)");
        assert_eq!(alloc::format!("{}", id("p")), "p");
        assert_eq!(id("p").as_str(), "p");
    }
}
