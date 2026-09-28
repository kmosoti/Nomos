//! A Condition cannot be fabricated around its constructor.
use nomos_core::condition::{Condition, FileCondition};
use nomos_core::resource::ResourcePath;

fn main() {
    let _ = Condition {
        path: ResourcePath::new("/etc/hosts").unwrap(),
        requirement: FileCondition::Absent,
    };
}
