//! Trace: the reconciliation loop with execution removed, run once (spec
//! §37). It is handed the observe capability only, so it cannot request a
//! mutation: N1 holds by construction, and `tests/compile-fail` shows the
//! attempt does not compile.

use nomos_core::assessment::Report;
use nomos_substrate::Observe;

use crate::kernel::Canon;

/// Observes every resource of `canon` once and assesses it.
pub fn trace(substrate: &mut impl Observe, canon: &Canon) -> Report {
    let observations = substrate.observe(&canon.keys());
    Report::assess(&canon.conditions(), &observations)
}
