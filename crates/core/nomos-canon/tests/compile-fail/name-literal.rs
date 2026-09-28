//! A name or label cannot be made from unchecked text.
use nomos_canon::model::Name;

fn main() {
    let _ = Name(String::from("Not A Name"));
}
