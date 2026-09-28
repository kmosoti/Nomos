//! A resource cannot be written as a literal, so no unvalidated label can
//! reach a Canon through one.
use std::collections::BTreeSet;

use nomos_canon::model::{Requirement, Resource, ServiceRequirement};

fn main() {
    let _ = Resource {
        requirement: Requirement::Service(ServiceRequirement::Running),
        keys: BTreeSet::new(),
        disrupts: BTreeSet::new(),
    };
}
