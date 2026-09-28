//! A Canon cannot be written as a literal: its fields are private.
use std::collections::{BTreeMap, BTreeSet};

use nomos_canon::model::{Canon, RawCanon};

fn main() {
    let valid = Canon::try_from(RawCanon {
        name: String::from("a"),
        ..RawCanon::default()
    })
    .unwrap();
    let _ = Canon {
        name: valid.name().clone(),
        resources: BTreeMap::new(),
        relations: BTreeSet::new(),
    };
}
