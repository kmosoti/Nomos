//! N1 (ADR 0013 §1): Trace is handed the observe capability only, so a
//! mutation requested through it does not compile.
use nomos_core::effect::Apply;
use nomos_substrate::Observe;

fn trace_that_mutates(substrate: &mut impl Observe, request: &Apply) {
    let _ = substrate.apply(request);
}

fn main() {}
