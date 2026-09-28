//! Failure-domain budgets over admission (N9, ADR 0010 §4).
//!
//! A budget bounds what admission may do, not what the world does: two
//! nodes in one rack can fail together and no admission prevented it. So
//! the predicate is over sets of nodes at the moment of admission
//! ([warp.md](../../../../docs/formal/warp.md#runnable-set)):
//!
//! $$
//! \mathrm{violates\_budget}(a) \iff
//! \exists f: \lvert U_f \cup R_f \cup D_f(a) \rvert > k_f
//! \ \vee\ \bigl(D(a) \neq \varnothing \wedge \neg\mathrm{Fresh}\bigr)
//! $$
//!
//! $U_f$ is the members of $f$ unavailable in the budget snapshot, $R_f$ the
//! members disrupted by reserved Actions and by those already chosen, and
//! $D_f(a)$ the members $a$ would disrupt. A node counts once however many
//! of the three sets hold it. An Action that disrupts nothing is never held
//! back; a stale snapshot holds back every Action that disrupts something.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// A node a disruptive Action can make unavailable.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Node(String);

impl Node {
    /// A node name, or `None` when `text` is empty.
    pub fn new(text: &str) -> Option<Self> {
        (!text.is_empty()).then(|| Node(String::from(text)))
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Node({})", self.0)
    }
}

/// One failure domain: its members and how many may be unavailable at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    members: BTreeSet<Node>,
    limit: usize,
}

impl Budget {
    /// A domain of `members` of which at most `limit` may be unavailable.
    pub fn new(members: BTreeSet<Node>, limit: usize) -> Self {
        Budget { members, limit }
    }

    /// The domain's members.
    pub fn members(&self) -> &BTreeSet<Node> {
        &self.members
    }

    /// The limit $k_f$.
    pub fn limit(&self) -> usize {
        self.limit
    }
}

/// Every budget, with the snapshot of unavailable nodes they are checked
/// against and whether that snapshot is fresh under the policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budgets {
    domains: Vec<Budget>,
    unavailable: BTreeSet<Node>,
    fresh: bool,
}

impl Budgets {
    /// No budgets: nothing is held back.
    pub fn none() -> Self {
        Budgets {
            domains: Vec::new(),
            unavailable: BTreeSet::new(),
            fresh: true,
        }
    }

    /// `domains`, checked against the nodes `unavailable` in a snapshot that
    /// is `fresh` or not. Freshness is decided by the caller, who has the
    /// clock and the policy; this module has neither.
    pub fn new(domains: Vec<Budget>, unavailable: BTreeSet<Node>, fresh: bool) -> Self {
        Budgets {
            domains,
            unavailable,
            fresh,
        }
    }

    /// The domains.
    pub fn domains(&self) -> &[Budget] {
        &self.domains
    }

    /// Whether admitting an Action that disrupts `disrupts`, beside the
    /// `reserved` disruption, would exceed a budget or rely on a stale
    /// snapshot.
    pub fn violated_by(&self, reserved: &BTreeSet<Node>, disrupts: &BTreeSet<Node>) -> bool {
        if disrupts.is_empty() {
            return false;
        }
        if !self.fresh {
            return true;
        }
        self.domains.iter().any(|domain| {
            let counted = domain
                .members
                .iter()
                .filter(|n| {
                    self.unavailable.contains(*n) || reserved.contains(*n) || disrupts.contains(*n)
                })
                .count();
            counted > domain.limit
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(names: &[&str]) -> BTreeSet<Node> {
        names.iter().map(|n| Node::new(n).unwrap()).collect()
    }

    fn rack() -> Budget {
        Budget::new(nodes(&["a", "b", "c"]), 1)
    }

    /// `budget-not-world`: the predicate counts the snapshot, the reserved
    /// set, and the candidate together, each node once.
    #[test]
    fn the_budget_counts_unavailable_reserved_and_disrupted_once_each() {
        let budgets = Budgets::new(vec_of(rack()), nodes(&[]), true);
        assert!(!budgets.violated_by(&nodes(&[]), &nodes(&["a"])));
        assert!(budgets.violated_by(&nodes(&["b"]), &nodes(&["a"])));
        assert!(!budgets.violated_by(&nodes(&["a"]), &nodes(&["a"])));
        let down = Budgets::new(vec_of(rack()), nodes(&["c"]), true);
        assert!(down.violated_by(&nodes(&[]), &nodes(&["a"])));
        assert!(!down.violated_by(&nodes(&[]), &nodes(&["c"])));
        assert!(!down.violated_by(&nodes(&[]), &nodes(&["z"])));
    }

    #[test]
    fn a_stale_snapshot_holds_back_only_disruptive_actions() {
        let stale = Budgets::new(vec_of(rack()), nodes(&[]), false);
        assert!(stale.violated_by(&nodes(&[]), &nodes(&["a"])));
        assert!(!stale.violated_by(&nodes(&[]), &nodes(&[])));
        assert!(!Budgets::none().violated_by(&nodes(&["a", "b"]), &nodes(&["c"])));
    }

    #[test]
    fn involuntary_failure_past_the_budget_pauses_disruption() {
        let over = Budgets::new(vec_of(rack()), nodes(&["a", "b"]), true);
        assert!(over.violated_by(&nodes(&[]), &nodes(&["a"])));
        assert!(over.violated_by(&nodes(&[]), &nodes(&["c"])));
        assert!(!over.violated_by(&nodes(&[]), &nodes(&[])));
    }

    #[test]
    fn names_say_what_they_are() {
        let n = Node::new("rack-1").unwrap();
        assert_eq!(n.as_str(), "rack-1");
        assert_eq!(alloc::format!("{n:?}"), "Node(rack-1)");
        assert!(Node::new("").is_none());
        assert_eq!(rack().limit(), 1);
        assert_eq!(rack().members().len(), 3);
        assert_eq!(
            Budgets::new(vec_of(rack()), nodes(&[]), true)
                .domains()
                .len(),
            1
        );
    }

    fn vec_of(b: Budget) -> Vec<Budget> {
        alloc::vec![b]
    }
}
