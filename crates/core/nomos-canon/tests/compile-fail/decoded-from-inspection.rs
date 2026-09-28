//! Archival inspection gives no Canon: an Inspection cannot be turned into
//! one.
use nomos_canon::artifact::{Inspection, Profile, inspect};
use nomos_canon::model::Canon;

fn main() {
    let i: Inspection = inspect(b"", Profile::Cbor).unwrap();
    let _: Canon = Canon::from(i);
}
