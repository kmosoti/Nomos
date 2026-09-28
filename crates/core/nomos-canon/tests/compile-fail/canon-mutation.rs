//! A validated Canon cannot be changed after validation: its accessors
//! give shared references only.
use nomos_canon::model::{Canon, RawCanon};

fn main() {
    let c = Canon::try_from(RawCanon {
        name: String::from("a"),
        ..RawCanon::default()
    })
    .unwrap();
    c.resources().clear();
}
