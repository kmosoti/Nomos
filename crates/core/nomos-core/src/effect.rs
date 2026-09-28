//! Effects as values: the key that names one execution, the request the
//! kernel issues, the receipt the Substrate returns, and settlement.
//!
//! Core performs no effect. It describes one as an [`EffectRequest`], and a
//! driver carries the request to a port (AGENTS.md rule 12). A receipt names
//! the execution it answers by [`EffectKey`], never by the content of the
//! Action: a key built from content would suppress every later repair of the
//! same drift (counterexample `dedup-scope`, ADR 0010 §1).
//!
//! **Settlement** is the working definition of ADR 0006's note, for the file
//! family and the service refresh: an effect is Settled by a terminal
//! receipt, or once its settle-by instant has passed, past which the
//! Substrate guarantees it can cause no further change. An acceptance or a
//! start receipt settles nothing, and neither does a satisfied Observation.

use alloc::vec::Vec;

use crate::condition::FileCondition;
use crate::observation::Instant;
use crate::plan::{Generation, PlanId};
use crate::resource::ResourcePath;

/// The idempotency key: one execution of one Action, within one iteration of
/// one Plan. Retries of the same delivery share it; the next iteration or
/// the next Plan gets a new one (ADR 0010 §1).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectKey {
    plan: PlanId,
    generation: Generation,
    iteration: u32,
    resource: ResourcePath,
}

impl EffectKey {
    /// The key for `resource`'s Action in `iteration` of `plan`.
    pub fn new(
        plan: PlanId,
        generation: Generation,
        iteration: u32,
        resource: ResourcePath,
    ) -> Self {
        EffectKey {
            plan,
            generation,
            iteration,
            resource,
        }
    }

    /// The Plan whose Action this is.
    pub fn plan(&self) -> &PlanId {
        &self.plan
    }

    /// The Plan's generation.
    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// The reconciliation iteration that planned the Action.
    pub fn iteration(&self) -> u32 {
        self.iteration
    }

    /// The resource the Action changes.
    pub fn resource(&self) -> &ResourcePath {
        &self.resource
    }
}

/// What an Action does to its resource.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Operation {
    /// Make the file satisfy this requirement.
    Replace(FileCondition),
    /// Restart the service the resource stands for, so that it loads its
    /// configuration again.
    Refresh,
}

/// A request to change one resource.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Apply {
    /// The execution this request is.
    pub key: EffectKey,
    /// What to do.
    pub operation: Operation,
    /// The instant after which the effect may cause no further change.
    pub settle_by: Instant,
}

/// An effect the kernel asks a driver to perform.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectRequest {
    /// Observe these resources.
    Observe(Vec<ResourcePath>),
    /// Change one resource.
    Apply(Apply),
}

/// What the Substrate reports about one execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Receipt {
    /// The request was accepted for execution.
    Accepted,
    /// The effect started.
    Started,
    /// The effect finished; `changed` says whether it altered the resource.
    Completed {
        /// Whether the resource differs from before.
        changed: bool,
    },
    /// The effect finished and failed.
    Failed,
    /// The request was refused; no effect started.
    Refused,
}

impl Receipt {
    /// Whether this receipt is settlement evidence: the outcome is known and
    /// the effect can cause no further change.
    pub fn settles(&self) -> bool {
        matches!(
            self,
            Receipt::Completed { .. } | Receipt::Failed | Receipt::Refused
        )
    }
}

/// Why an effect counts as Settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SettledBy {
    /// A terminal receipt arrived.
    Receipt,
    /// The settle-by instant passed.
    Deadline,
}

/// Whether an effect can still cause change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Settlement {
    /// The effect may still cause change until `settle_by`.
    Unsettled {
        /// The Substrate's bound on the effect.
        settle_by: Instant,
    },
    /// The effect can cause no further change.
    Settled(SettledBy),
}

impl Settlement {
    /// A new, unsettled effect bounded by `settle_by`.
    pub fn new(settle_by: Instant) -> Self {
        Settlement::Unsettled { settle_by }
    }

    /// Whether the effect is Settled.
    pub fn is_settled(&self) -> bool {
        matches!(self, Settlement::Settled(_))
    }

    /// Settlement after `receipt` arrives. Settled stays Settled.
    pub fn on_receipt(self, receipt: &Receipt) -> Self {
        match self {
            Settlement::Unsettled { .. } if receipt.settles() => {
                Settlement::Settled(SettledBy::Receipt)
            }
            other => other,
        }
    }

    /// Settlement at instant `now`. Settled stays Settled.
    pub fn at(self, now: Instant) -> Self {
        match self {
            Settlement::Unsettled { settle_by } if now >= settle_by => {
                Settlement::Settled(SettledBy::Deadline)
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECEIPTS: [Receipt; 6] = [
        Receipt::Accepted,
        Receipt::Started,
        Receipt::Completed { changed: true },
        Receipt::Completed { changed: false },
        Receipt::Failed,
        Receipt::Refused,
    ];

    /// The settlement table of ADR 0006's note, written out by hand.
    #[test]
    fn only_terminal_receipts_settle() {
        let expected = [false, false, true, true, true, true];
        for (receipt, settles) in RECEIPTS.iter().zip(expected) {
            assert_eq!(receipt.settles(), settles, "{receipt:?}");
            let after = Settlement::new(Instant(10)).on_receipt(receipt);
            assert_eq!(after.is_settled(), settles, "{receipt:?}");
        }
    }

    #[test]
    fn the_settle_by_instant_settles_and_nothing_earlier_does() {
        let unsettled = Settlement::new(Instant(10));
        assert_eq!(unsettled.at(Instant(9)), unsettled);
        assert_eq!(
            unsettled.at(Instant(10)),
            Settlement::Settled(SettledBy::Deadline)
        );
        assert_eq!(
            unsettled.at(Instant(11)),
            Settlement::Settled(SettledBy::Deadline)
        );
    }

    #[test]
    fn settled_stays_settled() {
        let settled = Settlement::new(Instant(10)).on_receipt(&Receipt::Failed);
        assert_eq!(settled, Settlement::Settled(SettledBy::Receipt));
        for receipt in RECEIPTS {
            assert_eq!(settled.on_receipt(&receipt), settled);
        }
        assert_eq!(settled.at(Instant(0)), settled);
        assert_eq!(settled.at(Instant(99)), settled);
    }

    /// `dedup-scope`: two iterations that plan the same repair of the same
    /// resource are two executions.
    #[test]
    fn a_key_names_an_execution_not_a_content() {
        let path = ResourcePath::new("/etc/app.conf").unwrap();
        let plan = PlanId::new("p").unwrap();
        let first = EffectKey::new(plan.clone(), Generation(1), 0, path.clone());
        let second = EffectKey::new(plan.clone(), Generation(1), 1, path.clone());
        let later = EffectKey::new(PlanId::new("q").unwrap(), Generation(2), 0, path);
        assert_ne!(first, second);
        assert_ne!(first, later);
        assert_eq!(first.plan(), &plan);
        assert_eq!(first.generation(), Generation(1));
        assert_eq!(first.iteration(), 0);
        assert_eq!(second.iteration(), 1);
        assert_eq!(first.resource().as_str(), "/etc/app.conf");
    }
}
